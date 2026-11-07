//! Client for the speech-to-text server on the 16 GB Pi. It speaks the OpenAI
//! `/audio/transcriptions` wire format through Rig's `TranscriptionModel`.
//!
//! Audio stays in memory: it is wrapped as WAV, sent over the LAN, and dropped.

use std::{io::Cursor, time::Duration};

use reqwest::{StatusCode, Url, redirect::Policy};
use rig_core::{
    http_client,
    prelude::TranscriptionClient,
    providers::openai,
    transcription::{TranscriptionError, TranscriptionModel},
};
use serde::Deserialize;

use super::local_endpoint::{LocalEndpointError, LocalEndpointResolver, validate_local_endpoint};
use crate::config::SpeechToTextConfig;

/// Bounds one transcription request, including reading the response. Rig has no
/// per-request timeout, so it lives on the HTTP client. Jarvis stops waiting;
/// the server may still be working.
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(20);
/// Bounds the `/health` probe.
const TRANSCRIPTION_HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
/// The longest transcript accepted, in bytes. It matches `MAX_INPUT_BYTES` in
/// `interfaces/cli/input.rs` (a transcript is handled like typed input), which
/// is private to the CLI and which `clients` must not import.
const MAX_TRANSCRIPT_BYTES: usize = 2_048;
/// Longest server error message kept in an error, in characters.
const MAX_SERVER_MESSAGE_CHARS: usize = 200;
/// The capture format fixed by `features/listening`: 16 kHz mono signed 16-bit.
const SAMPLE_RATE_HZ: u32 = 16_000;
const WAV_FILENAME: &str = "audio.wav";
/// Sent so the server does not guess the language (D14: no prompt is sent).
const LANGUAGE: &str = "en";

#[derive(Debug, thiserror::Error)]
pub(crate) enum SpeechToTextError {
    #[error("speech-to-text server URL is not allowed")]
    InvalidEndpoint(#[from] LocalEndpointError),
    #[error("could not build the speech-to-text HTTP client")]
    HttpClient(#[source] reqwest::Error),
    #[error("could not build the speech-to-text provider client")]
    Provider(#[source] http_client::Error),
    #[error("could not encode the recording as WAV")]
    Encode(#[source] hound::Error),
    #[error("stopped waiting for the speech-to-text server; it may still be working")]
    TimedOut(#[source] TranscriptionError),
    #[error("could not reach the speech-to-text server")]
    Connection(#[source] TranscriptionError),
    #[error("speech-to-text server returned {status}: {message}")]
    Server {
        status: StatusCode,
        message: String,
        #[source]
        source: TranscriptionError,
    },
    #[error("speech-to-text server sent a response that could not be used")]
    InvalidResponse(#[source] TranscriptionError),
    #[error("transcript is {bytes} bytes, over the {MAX_TRANSCRIPT_BYTES} byte limit")]
    TranscriptTooLong { bytes: usize },
    #[error("speech-to-text health check failed")]
    Health(#[source] reqwest::Error),
}

/// What the server heard.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Transcript {
    /// Trimmed, non-empty, at most `MAX_TRANSCRIPT_BYTES`.
    Text(String),
    /// Blank or whitespace-only: the "didn't catch that" case.
    Nothing,
}

/// OpenAI-style error envelope: `{"error": {"message": "..."}}`.
#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

#[derive(Clone)]
pub(crate) struct SpeechToText {
    model: openai::CompletionsTranscriptionModel,
    health_http: reqwest::Client,
    health_url: Url,
}

impl SpeechToText {
    pub(crate) fn new(config: &SpeechToTextConfig) -> Result<Self, SpeechToTextError> {
        Self::with_timeout(config, TRANSCRIPTION_TIMEOUT)
    }

    fn with_timeout(
        config: &SpeechToTextConfig,
        timeout: Duration,
    ) -> Result<Self, SpeechToTextError> {
        let base_url = validate_local_endpoint(&config.base_url)?;
        let health_url = base_url
            .join("/health")
            .map_err(|_| LocalEndpointError::InvalidUrl)?;

        let transcription_http = reqwest::Client::builder()
            .timeout(timeout)
            .retry(reqwest::retry::never())
            .no_proxy()
            .dns_resolver(LocalEndpointResolver)
            .redirect(Policy::none())
            .build()
            .map_err(SpeechToTextError::HttpClient)?;
        let client = openai::CompletionsClient::builder()
            .api_key("local")
            .base_url(base_url.as_str())
            .http_client(transcription_http)
            .build()
            .map_err(SpeechToTextError::Provider)?;
        let health_http = reqwest::Client::builder()
            .timeout(TRANSCRIPTION_HEALTH_TIMEOUT)
            .no_proxy()
            .dns_resolver(LocalEndpointResolver)
            .redirect(Policy::none())
            .build()
            .map_err(SpeechToTextError::HttpClient)?;

        Ok(Self {
            model: client.transcription_model(&config.model),
            health_http,
            health_url,
        })
    }

    /// Probes `GET /health` at the server root (not under `/v1`).
    pub(crate) async fn health_check(&self) -> Result<(), SpeechToTextError> {
        self.health_http
            .get(self.health_url.clone())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(SpeechToTextError::Health)?;
        Ok(())
    }

    /// Transcribes mono 16 kHz signed 16-bit little-endian PCM, as captured by
    /// `features/listening`. A trailing odd byte is not a whole sample and is
    /// dropped. Nothing is retried: a timeout does not prove the server stopped.
    pub(crate) async fn transcribe(&self, pcm: &[u8]) -> Result<Transcript, SpeechToTextError> {
        let wav = wav_from_pcm(pcm).map_err(SpeechToTextError::Encode)?;
        let response = self
            .model
            .transcription_request()
            .data(wav)
            .filename(Some(WAV_FILENAME.to_owned()))
            .language(LANGUAGE.to_owned())
            .send()
            .await
            .map_err(classify)?;

        let text = response.text.trim();
        if text.is_empty() {
            return Ok(Transcript::Nothing);
        }
        if text.len() > MAX_TRANSCRIPT_BYTES {
            return Err(SpeechToTextError::TranscriptTooLong { bytes: text.len() });
        }
        Ok(Transcript::Text(text.to_owned()))
    }
}

fn wav_from_pcm(pcm: &[u8]) -> Result<Vec<u8>, hound::Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(pcm.len() + 44));
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    let (samples, _odd_byte) = pcm.as_chunks::<2>();
    for sample in samples {
        writer.write_sample(i16::from_le_bytes(*sample))?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

/// Sorts Rig's one error type by what the caller can do about it.
fn classify(error: TranscriptionError) -> SpeechToTextError {
    if let TranscriptionError::HttpError(http_client::Error::Instance(source)) = &error {
        return match source.downcast_ref::<reqwest::Error>() {
            Some(source) if source.is_timeout() => SpeechToTextError::TimedOut(error),
            _ => SpeechToTextError::Connection(error),
        };
    }
    if let Some(status) = error.provider_response_status() {
        let body = error.provider_response_body().unwrap_or_default();
        // A 2xx reply is only the server's error when it is an error envelope;
        // any other 2xx body that Rig refused is simply unusable.
        let message = if status.is_success() {
            envelope_message(body)
        } else {
            Some(server_message(body))
        };
        if let Some(message) = message {
            return SpeechToTextError::Server {
                status,
                message: truncated(message),
                source: error,
            };
        }
    }
    SpeechToTextError::InvalidResponse(error)
}

fn envelope_message(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .map(|envelope| envelope.error.message)
}

/// The envelope's message, or the raw body when it is not an envelope.
fn server_message(body: &str) -> String {
    envelope_message(body).unwrap_or_else(|| body.trim().to_owned())
}

/// Capped so a verbose server cannot flood logs.
fn truncated(message: String) -> String {
    message.chars().take(MAX_SERVER_MESSAGE_CHARS).collect()
}

#[cfg(test)]
#[path = "../../tests/unit/clients/speech_to_text_tests.rs"]
mod tests;
