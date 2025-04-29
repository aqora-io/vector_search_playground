use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::Lazy;
use qdrant_client::config::CompressionEncoding;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, ScalarQuantizationBuilder, SearchParamsBuilder,
    SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::{Payload, Qdrant};
use sea_orm::{
    prelude::PgVector, ActiveValue::Set, ConnectOptions, Database, EntityTrait, PaginatorTrait,
};

use serde::Serialize;
use uuid::Uuid;

const DEFAULT_COLLECTION_NAME: &str = "search";
const MODEL_DIM: u64 = 384;

static EMBEDDER: Lazy<TextEmbedding> = Lazy::new(|| {
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false),
    )
    .expect("embedder init")
});

#[derive(Args, Debug, Serialize, Clone)]
pub struct CliArgs {
    #[arg(short = 'd', long, env = "DATABASE_URL")]
    pub database_url: url::Url,
    #[arg(short = 'q', long, env = "QDRANT_URL")]
    pub qdrant_url: url::Url,
    #[arg(short = 't', long, default_value = "0.6")]
    pub threashold: Option<f32>,
}

#[derive(Parser, Debug, Serialize)]
#[command(author, about)]
pub struct Cli {
    #[command(flatten)]
    pub cliargs: CliArgs,
    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Args, Debug, Serialize)]
#[command(author, version, about)]
pub struct Create {
    pub content: String,
}

#[derive(Args, Debug, Serialize)]
#[command(author, version, about)]
pub struct Search {
    pub query: String,
    #[arg(long, default_value_t = 10)]
    pub top_k: u64,
}

#[derive(Subcommand, Debug, Serialize)]
pub enum Commands {
    Collections,
    Create(Create),
    Count,
    Search(Search),
}

fn embed_batch<S: AsRef<str> + Send + Sync>(texts: Vec<S>) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    EMBEDDER.embed(texts, Some(256))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let db_conn = {
        let connect_opts = ConnectOptions::from(args.cliargs.database_url);
        Database::connect(connect_opts).await?
    };

    let client = Qdrant::from_url(args.cliargs.qdrant_url.as_str())
        .compression(Some(CompressionEncoding::Gzip))
        .build()?;

    match args.commands {
        Commands::Collections => {
            let collections = client.list_collections().await?;
            println!("collections: {:?}", collections);
        }
        Commands::Create(create) => {
            let collection_exist = client.collection_exists(DEFAULT_COLLECTION_NAME).await?;

            if !collection_exist {
                client
                    .create_collection(
                        CreateCollectionBuilder::new(DEFAULT_COLLECTION_NAME)
                            .vectors_config(VectorParamsBuilder::new(MODEL_DIM, Distance::Cosine))
                            .quantization_config(ScalarQuantizationBuilder::default()),
                    )
                    .await?;
            }

            let embedding = embed_batch(vec![&create.content])?.pop().unwrap();
            let uuid = Uuid::now_v7();

            let payload: Payload = serde_json::json!(
                {
                    "id": uuid,
                    "content": &create.content,
                }
            )
            .try_into()
            .unwrap();

            let search = entity::search::ActiveModel {
                id: Set(uuid),
                vector: Set(PgVector::from(embedding.clone())),
                content: Set(create.content),
            };
            entity::search::Entity::insert(search)
                .exec(&db_conn)
                .await?;

            client
                .upsert_points(UpsertPointsBuilder::new(
                    DEFAULT_COLLECTION_NAME,
                    vec![PointStruct::new(
                        Uuid::now_v7().to_string(),
                        embedding,
                        payload,
                    )],
                ))
                .await?;

            println!("done.");
        }
        Commands::Count => {
            let search_count = entity::search::Entity::find().count(&db_conn).await?;
            println!("rows: {}", search_count);
        }
        Commands::Search(search) => {
            let search_result = client
                .search_points(
                    SearchPointsBuilder::new(
                        DEFAULT_COLLECTION_NAME,
                        embed_batch(vec![&search.query])?.pop().unwrap(),
                        search.top_k,
                    )
                    .params(SearchParamsBuilder::default().hnsw_ef(128)),
                )
                .await?;

            println!("search matches: {:?}", search_result);
        }
    }
    db_conn.close().await?;
    Ok(())
}
