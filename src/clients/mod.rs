mod local_endpoint;
pub mod local_llm;
pub(crate) mod remote_embeddings;
// T6 wires the client into `/listen`; remove this allow then.
#[allow(dead_code)]
pub(crate) mod speech_to_text;
