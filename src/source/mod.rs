use async_trait::async_trait;

use crate::models::{Episode, Meta, Movie, SearchMode, Stream};

pub mod animeku;
pub mod embed;
pub mod lk21;
pub mod otakudesu;
pub mod tokusatsu;

#[async_trait]
pub trait Source {
    fn name(&self) -> &'static str;

    async fn search(&mut self, query: &str, mode: SearchMode) -> anyhow::Result<Vec<Movie>>;
    async fn episodes(&self, movie: &Movie) -> anyhow::Result<(Vec<Episode>, Meta)>;
    async fn streams(&self, episode: &Episode) -> anyhow::Result<Vec<Stream>>;
    async fn resolve(&self, stream: &Stream) -> anyhow::Result<Option<String>>;
}
