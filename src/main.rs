pub mod activity_graph;
mod app;
pub mod bar;
mod bot;
mod commands;
mod config;
mod db;
mod domain;
mod error;
mod state;
pub mod stats;
pub mod terminal;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
