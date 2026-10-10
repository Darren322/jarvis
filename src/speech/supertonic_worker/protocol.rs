use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum WorkerRequest {
    #[serde(rename = "speak")]
    Speak {
        protocol: u32,
        id: String,
        text: String,
        output_path: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerResponse {
    #[serde(rename = "ready")]
    Ready {
        protocol: u32,
        pid: u32,
        engine: String,
        precision: String,
        voice: String,
        language: String,
        provider: String,
        threads: u32,
        sample_rate: u32,
        num_speakers: u32,
    },

    #[serde(rename = "completed")]
    Completed {
        protocol: u32,
        id: String,
        sample_rate: u32,
        num_samples: usize,
        wav_bytes: usize,
    },

    #[serde(rename = "error")]
    Error {
        protocol: u32,
        id: Option<String>,
        #[serde(rename = "error")]
        _error: String,
    },
}
