#[tokio::main]
async fn main() {
    println!("Starting jarvis.....");

    let response = reqwest::get("http://jarvis-ai.local:8080/health").await;

    print!("{:?}", response);
}
