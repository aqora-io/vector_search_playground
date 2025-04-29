use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use elasticsearch::{
    cat::CatIndicesParts,
    http::transport::Transport,
    indices::{IndicesCreateParts, IndicesExistsParts},
    Elasticsearch, IndexParts, SearchParts,
};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::Lazy;
use sea_orm::{
    prelude::PgVector, ActiveValue::Set, ConnectOptions, Database, EntityTrait, PaginatorTrait,
};
use serde::Serialize;
use serde_json::json;
use url::Url;
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
    pub database_url: Url,
    #[arg(short = 'e', long, env = "ELASTIC_URL")]
    pub elastic_url: Url,
    #[arg(short = 't', long, default_value = "0.6")]
    pub threshold: Option<f32>,
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

async fn es_client(url: &Url) -> Result<Elasticsearch> {
    let transport = Transport::single_node(url.as_str())?;
    Ok(Elasticsearch::new(transport))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let db_conn = {
        let connect_opts = ConnectOptions::from(args.cliargs.database_url.clone());
        Database::connect(connect_opts).await?
    };

    let es = es_client(&args.cliargs.elastic_url).await?;

    match args.commands {
        Commands::Collections => {
            let cat = es
                .cat()
                .indices(CatIndicesParts::None)
                .format("json")
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;
            println!("indices: {cat:#}");
        }
        Commands::Create(create) => {
            let index_exists = es
                .indices()
                .exists(IndicesExistsParts::Index(&[DEFAULT_COLLECTION_NAME]))
                .send()
                .await?
                .status_code()
                .is_success();

            if !index_exists {
                es.indices()
                    .create(IndicesCreateParts::Index(DEFAULT_COLLECTION_NAME))
                    .body(json!({
                        "settings": {"index": {"knn": true}},
                        "mappings": {
                            "properties": {
                                "vector": {"type": "dense_vector", "dims": MODEL_DIM, "index": true, "similarity": "cosine"},
                                "content": {"type": "text"}
                            }
                        }
                    }))
                    .send()
                    .await?;
            }

            let embedding = embed_batch(vec![&create.content])?.pop().unwrap();
            let uuid = Uuid::now_v7();

            let search_row = entity::search::ActiveModel {
                id: Set(uuid),
                vector: Set(PgVector::from(embedding.clone())),
                content: Set(create.content.clone()),
            };
            entity::search::Entity::insert(search_row)
                .exec(&db_conn)
                .await?;

            es.index(IndexParts::IndexId(
                DEFAULT_COLLECTION_NAME,
                &uuid.to_string(),
            ))
            .body(json!({"vector": embedding, "content": &create.content}))
            .send()
            .await?;

            println!("done.");
        }
        Commands::Count => {
            let search_count = entity::search::Entity::find().count(&db_conn).await?;
            println!("rows: {}", search_count);
        }
        Commands::Search(search) => {
            let query_vec = embed_batch(vec![&search.query])?.pop().unwrap();

            let resp = es
                .search(SearchParts::Index(&[DEFAULT_COLLECTION_NAME]))
                .body(json!({
                    "knn": {
                        "field": "vector",
                        "query_vector": query_vec,
                        "k": search.top_k,
                        "num_candidates": 100
                    },
                    "_source": {"includes": ["content"]}
                }))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            println!("matches: {resp:#}");
        }
    }

    db_conn.close().await?;
    Ok(())
}
