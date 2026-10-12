//! Compile/run checkpoint for the local embedding and embedded LanceDB dependencies.

#[path = "../src/features/memory/backend_probe.rs"]
mod backend_probe;
#[path = "../src/clients/local_embeddings.rs"]
mod local_embeddings;

use std::{env, error::Error, path::Path};

use local_embeddings::LocalEmbeddings;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!(
        "local_embedding_model_fingerprint={}",
        local_embeddings::MODEL_FINGERPRINT
    );
    let hybrid_rows = backend_probe::run_local_hybrid_probe().await?;
    println!("native_hybrid_ids={hybrid_rows:?}");
    for row in hybrid_rows {
        println!("native_hybrid_result_id={row}");
    }

    if let Some(model_dir) = env::var_os("JARVIS_EMBEDDING_MODEL_DIR") {
        let model = LocalEmbeddings::load_from_dir(Path::new(&model_dir))?;
        let query = model.embed_query("Which city did I travel to by train last weekend?")?;
        let documents = [
            "Last weekend, I took a train to Kyoto for a short visit.",
            "I made vegetable soup for dinner yesterday.",
            "Paris is the capital of France.",
        ];
        let vectors = documents
            .iter()
            .map(|document| model.embed_document(document))
            .collect::<Result<Vec<_>, _>>()?;
        let scores = vectors
            .iter()
            .map(|vector| cosine_from_unit_vectors(&query, vector))
            .collect::<Vec<_>>();
        let best_match = scores
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .ok_or_else(|| std::io::Error::other("the probe must include at least one passage"))?;

        println!("local_embedding_dimensions={}", query.len());
        println!("semantic_probe_best_match={best_match}");
        println!("semantic_probe_scores={scores:?}");
        if best_match != 0 {
            return Err(std::io::Error::other(
                "the paraphrase probe did not rank the Kyoto passage first",
            )
            .into());
        }
    }

    Ok(())
}

fn cosine_from_unit_vectors(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}
