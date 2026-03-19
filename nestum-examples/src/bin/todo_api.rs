use nestum_examples::todo_api;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = todo_api::app::State::in_memory().await?;
    let mut events = state.subscribe();

    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            eprintln!("{}", event.summary());
        }
    });

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    println!("todo_api listening on http://127.0.0.1:3000");
    axum::serve(listener, todo_api::router(state)).await?;
    Ok(())
}
