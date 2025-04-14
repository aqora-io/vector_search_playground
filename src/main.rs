use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use migration::IntoCondition;
use sea_orm::sea_query::extension::postgres::PgBinOper;
use sea_orm::sea_query::ExprTrait;
use sea_orm::sea_query::{Expr, Order};
use sea_orm::QueryOrder;
use sea_orm::{
    prelude::PgVector,
    ActiveValue::{NotSet, Set},
    ConnectOptions, Database, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect,
};
use serde::Serialize;

#[derive(Args, Debug, Serialize, Clone)]
pub struct CliArgs {
    #[arg(short = 'd', long, env = "DATABASE_URL")]
    pub database_url: url::Url,
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
}

#[derive(Subcommand, Debug, Serialize)]
pub enum Commands {
    Create(Create),
    Count,
    Search(Search),
}

fn create_embedding(content: impl AsRef<str> + Send + Sync) -> Result<PgVector> {
    Ok(PgVector::from(
        TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?
            .embed(vec![content], None)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No embed"))?,
    ))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let db_conn = {
        let connect_opts = ConnectOptions::from(args.cliargs.database_url);
        Database::connect(connect_opts).await?
    };

    match args.commands {
        Commands::Create(create) => {
            entity::search::Entity::insert(entity::search::ActiveModel {
                id: NotSet,
                vector: Set(create_embedding(&create.content)?),
                content: Set(create.content),
            })
            .exec(&db_conn)
            .await?;
        }
        Commands::Count => {
            let search_count = entity::search::Entity::find().count(&db_conn).await?;
            println!("rows: {}", search_count);
        }
        Commands::Search(search) => {
            let expr = Expr::col(entity::search::Column::Vector)
                .binary(PgBinOper::CosineDistance, create_embedding(&search.query)?);
            entity::search::Entity::find()
                .filter(expr.clone().lt(args.cliargs.threashold))
                .order_by(expr, Order::Asc)
                .limit(10)
                .all(&db_conn)
                .await?
                .into_iter()
                .for_each(|search| println!("{:?}", search.content))
        }
    }
    db_conn.close().await?;
    Ok(())
}
