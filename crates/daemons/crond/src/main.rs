use guilderia_config::configure;
use guilderia_database::DatabaseInfo;
use guilderia_result::Result;
use tasks::{file_deletion, prune_dangling_files};
use tokio::try_join;

pub mod tasks;

#[tokio::main]
async fn main() -> Result<()> {
    configure!(crond);

    let db = DatabaseInfo::Auto.connect().await.expect("database");
    try_join!(
        file_deletion::task(db.clone()),
        prune_dangling_files::task(db)
    )
    .map(|_| ())
}
