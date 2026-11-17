use crate::rise_store::rise_sq_lite::RiseSQLiteConnection;
use crate::rise_store::rise_store_models::RiseStoreError;

fn scalar(database: &RiseSQLiteConnection, sql: &str) -> Result<i64, RiseStoreError> {
    let mut statement = database.statement(sql, "cache.test")?;
    assert!(statement.step()?);
    let value = statement.int64(0);
    statement.release();
    Ok(value)
}

#[test]
fn reuses_prepared_statements_and_evicts_the_least_recently_used_entry() -> Result<(), RiseStoreError> {
    let mut database = RiseSQLiteConnection::new_with_statement_cache_capacity(":memory:", 2)?;

    assert_eq!(scalar(&database, "SELECT 1")?, 1);
    assert_eq!(database.prepared_statement_count_for_testing(), 1);
    assert_eq!(scalar(&database, "SELECT 1")?, 1);
    assert_eq!(database.prepared_statement_count_for_testing(), 1);

    assert_eq!(scalar(&database, "SELECT 2")?, 2);
    assert_eq!(scalar(&database, "SELECT 1")?, 1);
    assert_eq!(scalar(&database, "SELECT 3")?, 3);
    assert_eq!(database.prepared_statement_count_for_testing(), 3);
    assert_eq!(database.cached_statement_count_for_testing(), 2);

    assert_eq!(scalar(&database, "SELECT 2")?, 2);
    assert_eq!(database.prepared_statement_count_for_testing(), 4);
    assert_eq!(database.cached_statement_count_for_testing(), 2);
    database.close()?;
    Ok(())
}

#[test]
fn overlapping_use_of_the_same_sql_gets_a_transient_statement() -> Result<(), RiseStoreError> {
    let database = RiseSQLiteConnection::new_with_statement_cache_capacity(":memory:", 2)?;

    let mut cached_lease = database.statement("SELECT 42", "cache.primary")?;
    let mut transient_lease = database.statement("SELECT 42", "cache.transient")?;
    assert_eq!(database.prepared_statement_count_for_testing(), 2);
    assert!(cached_lease.step()?);
    assert!(transient_lease.step()?);
    assert_eq!(cached_lease.int64(0), 42);
    assert_eq!(transient_lease.int64(0), 42);

    cached_lease.release();
    transient_lease.release();
    assert_eq!(database.cached_statement_count_for_testing(), 1);

    let mut reused = database.statement("SELECT 42", "cache.reused")?;
    assert!(reused.step()?);
    assert_eq!(database.prepared_statement_count_for_testing(), 2);
    reused.release();
    Ok(())
}

#[test]
fn resets_and_clears_bindings_before_reuse() -> Result<(), RiseStoreError> {
    let database = RiseSQLiteConnection::new_with_statement_cache_capacity(":memory:", 1)?;

    let mut first = database.statement("SELECT ?", "cache.bound")?;
    first.bind(99_i64, 1)?;
    assert!(first.step()?);
    assert_eq!(first.int64(0), 99);
    first.release();

    let mut reused = database.statement("SELECT ?", "cache.cleared")?;
    assert!(reused.step()?);
    assert!(reused.is_null(0));
    assert_eq!(database.prepared_statement_count_for_testing(), 1);
    reused.release();
    Ok(())
}
