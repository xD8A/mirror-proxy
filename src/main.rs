use std::io::Write;
use log::info;


#[tokio::main]
async fn main() {
    // setup logging
    env_logger::Builder::new()
        .format(|in_buf, record| {
            writeln!(in_buf,
                     "{} [{}] - {}",
                     chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
                     record.level(),
                     record.args()
            )
        })
        .filter(None, log::LevelFilter::Debug)
        .init();

    info!("Server listening at 127.0.0.1:8080");
}
