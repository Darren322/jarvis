pub(crate) mod local_embeddings;
pub mod local_llm;

#[cfg(test)]
#[path = "../../tests/unit/clients/openai_adapter_tests.rs"]
mod openai_adapter_tests;
