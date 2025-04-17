use sea_orm::{sea_query::extension::postgres::Extension, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;

        tx.execute(sea_orm::Statement::from_string(
            manager.get_database_backend(),
            Extension::create()
                .name("vector")
                .cascade()
                .if_not_exists()
                .to_string(PostgresQueryBuilder),
        ))
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(Search::Table)
                    .col(
                        ColumnDef::new(Search::Id)
                            .integer()
                            .not_null()
                            .primary_key()
                            .auto_increment(),
                    )
                    .col(ColumnDef::new(Search::Content).string().not_null())
                    .col(ColumnDef::new(Search::Vector).vector(None).not_null())
                    .to_owned(),
            )
            .await?;

        tx.commit().await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;

        tx.execute(sea_orm::Statement::from_string(
            manager.get_database_backend(),
            Extension::drop()
                .name("vector")
                .cascade()
                .to_string(PostgresQueryBuilder),
        ))
        .await?;

        manager
            .drop_table(Table::drop().table(Search::Table).to_owned())
            .await?;

        tx.commit().await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Search {
    Table,
    Id,
    Content,
    Vector,
}
