pub async fn health_check() {
    let response = reqwest::get("http://jarvis-ai.local:8080/health").await;

    match response {
        Ok(res) => {
            println!("jarvis-ai is online");
            println!("Status: {}", res.status());
        }

        Err(err) => {
            println!("jarvis-ai is offline");
            println!("Error: {}", err);
        }
    }
}
