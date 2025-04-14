pub use sea_orm_migration::prelude::*;

mod m20250414_131949_create_search;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20250414_131949_create_search::Migration)]
    }
}
