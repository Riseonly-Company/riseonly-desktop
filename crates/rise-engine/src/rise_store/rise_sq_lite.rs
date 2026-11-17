use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use rusqlite::types::{Null, ToSql, ToSqlOutput, Value, ValueRef};
use rusqlite::{CachedStatement, Connection, OpenFlags, Statement};

use crate::rise_store::rise_store_models::RiseStoreError;

const RISE_SQ_LITE_DEFAULT_STATEMENT_CACHE_CAPACITY: i64 = 96;
const RISE_SQ_LITE_BUSY_TIMEOUT_MILLISECONDS: u64 = 2_000;

struct RiseSQLiteConnectionCachedStatement {
    last_use: u64,
}

pub(crate) struct RiseSQLiteConnection {
    cache_capacity: usize,
    cached_statements: RefCell<HashMap<String, RiseSQLiteConnectionCachedStatement>>,
    cacheable_statements_in_use: RefCell<HashSet<String>>,
    cache_clock: Cell<u64>,
    preparation_count: Cell<u64>,
    handle: Option<Connection>,
}

impl RiseSQLiteConnection {
    pub(crate) fn new_with_statement_cache_capacity(
        path: &str,
        statement_cache_capacity: i64,
    ) -> Result<Self, RiseStoreError> {
        let flags = OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        let database = Connection::open_with_flags(path, flags)
            .map_err(|error| rise_sq_lite_error(&error, "open"))?;
        let cache_capacity = usize::try_from(statement_cache_capacity.max(0)).unwrap_or(usize::MAX);
        database.set_prepared_statement_cache_capacity(cache_capacity);
        database
            .busy_timeout(Duration::from_millis(RISE_SQ_LITE_BUSY_TIMEOUT_MILLISECONDS))
            .map_err(|error| rise_sq_lite_error(&error, "open"))?;
        Ok(Self {
            cache_capacity,
            cached_statements: RefCell::new(HashMap::new()),
            cacheable_statements_in_use: RefCell::new(HashSet::new()),
            cache_clock: Cell::new(0),
            preparation_count: Cell::new(0),
            handle: Some(database),
        })
    }

    pub(crate) fn new(path: &str) -> Result<Self, RiseStoreError> {
        Self::new_with_statement_cache_capacity(path, RISE_SQ_LITE_DEFAULT_STATEMENT_CACHE_CAPACITY)
    }

    pub(crate) fn close(&mut self) -> Result<(), RiseStoreError> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        self.finalize_cached_statements_locked(&handle);
        match handle.close() {
            Ok(()) => Ok(()),
            Err((handle, error)) => {
                self.handle = Some(handle);
                Err(rise_sq_lite_error(&error, "close"))
            }
        }
    }

    pub(crate) fn execute(&self, sql: &str, operation: &str) -> Result<(), RiseStoreError> {
        let Some(handle) = self.handle.as_ref() else {
            return Err(RiseStoreError::NotOpen);
        };
        handle
            .execute_batch(sql)
            .map_err(|error| rise_sq_lite_error(&error, operation))
    }

    pub(crate) fn statement(
        &self,
        sql: &str,
        operation: &str,
    ) -> Result<RiseSQLiteStatement<'_>, RiseStoreError> {
        let Some(handle) = self.handle.as_ref() else {
            return Err(RiseStoreError::NotOpen);
        };

        if self.cached_statements.borrow_mut().remove(sql).is_some() {
            self.cacheable_statements_in_use
                .borrow_mut()
                .insert(sql.to_owned());
            return match handle.prepare_cached(sql) {
                Ok(statement) => Ok(RiseSQLiteStatement::new(
                    RiseSQLiteStatementPointer::Cached(statement),
                    operation,
                    self,
                    sql,
                    true,
                )),
                Err(error) => {
                    self.cacheable_statements_in_use.borrow_mut().remove(sql);
                    Err(rise_sq_lite_error(&error, operation))
                }
            };
        }

        let returns_to_cache = self
            .cacheable_statements_in_use
            .borrow_mut()
            .insert(sql.to_owned());
        self.preparation_count
            .set(self.preparation_count.get().wrapping_add(1));
        let prepared = if returns_to_cache {
            handle
                .prepare_cached(sql)
                .map(RiseSQLiteStatementPointer::Cached)
        } else {
            handle
                .prepare(sql)
                .map(RiseSQLiteStatementPointer::Transient)
        };
        match prepared {
            Ok(pointer) => Ok(RiseSQLiteStatement::new(
                pointer,
                operation,
                self,
                sql,
                returns_to_cache,
            )),
            Err(error) => {
                if returns_to_cache {
                    self.cacheable_statements_in_use.borrow_mut().remove(sql);
                }
                Err(rise_sq_lite_error(&error, operation))
            }
        }
    }

    pub(crate) fn changed_rows(&self) -> i32 {
        let Some(handle) = self.handle.as_ref() else {
            return 0;
        };
        i32::try_from(handle.changes()).unwrap_or(i32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn prepared_statement_count_for_testing(&self) -> u64 {
        self.preparation_count.get()
    }

    #[cfg(test)]
    pub(crate) fn cached_statement_count_for_testing(&self) -> usize {
        self.cached_statements.borrow().len()
    }

    fn return_statement(&self, cache_key: &str, returns_to_cache: bool) {
        if !returns_to_cache {
            return;
        }

        self.cacheable_statements_in_use
            .borrow_mut()
            .remove(cache_key);
        if self.handle.is_none() || self.cache_capacity == 0 {
            return;
        }

        let clock = self.cache_clock.get().wrapping_add(1);
        self.cache_clock.set(clock);
        self.cached_statements.borrow_mut().insert(
            cache_key.to_owned(),
            RiseSQLiteConnectionCachedStatement { last_use: clock },
        );
        self.evict_cached_statements_if_needed_locked();
    }

    fn evict_cached_statements_if_needed_locked(&self) {
        let mut cached_statements = self.cached_statements.borrow_mut();
        while cached_statements.len() > self.cache_capacity {
            let Some(oldest) = cached_statements
                .iter()
                .min_by_key(|(_, statement)| statement.last_use)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cached_statements.remove(&oldest);
        }
    }

    fn finalize_cached_statements_locked(&self, handle: &Connection) {
        self.cached_statements.borrow_mut().clear();
        handle.flush_prepared_statement_cache();
    }
}

enum RiseSQLiteStatementPointer<'connection> {
    Cached(CachedStatement<'connection>),
    Transient(Statement<'connection>),
}

impl<'connection> RiseSQLiteStatementPointer<'connection> {
    fn statement(&mut self) -> &mut Statement<'connection> {
        match self {
            Self::Cached(statement) => statement,
            Self::Transient(statement) => statement,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RiseSQLiteValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

impl RiseSQLiteValue {
    fn from_value_ref(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(integer) => Self::Integer(integer),
            ValueRef::Real(real) => Self::Real(real),
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }

    fn text_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Null => None,
            Self::Integer(integer) => Some(integer.to_string().into_bytes()),
            Self::Real(real) => Some(format!("{real:?}").into_bytes()),
            Self::Text(bytes) | Self::Blob(bytes) => Some(bytes.clone()),
        }
    }

    fn byte_count(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::Text(bytes) | Self::Blob(bytes) => bytes.len(),
            Self::Integer(_) | Self::Real(_) => self.text_bytes().map_or(0, |bytes| bytes.len()),
        }
    }

    fn int64(&self) -> i64 {
        match self {
            Self::Null => 0,
            Self::Integer(integer) => *integer,
            Self::Real(real) => *real as i64,
            Self::Text(bytes) | Self::Blob(bytes) => rise_sq_lite_leading_integer(bytes),
        }
    }
}

pub(crate) struct RiseSQLiteStatement<'connection> {
    pointer: Option<RiseSQLiteStatementPointer<'connection>>,
    operation: String,
    owner: &'connection RiseSQLiteConnection,
    cache_key: String,
    returns_to_cache: bool,
    pending_rows: Option<VecDeque<Vec<RiseSQLiteValue>>>,
    current_row: Option<Vec<RiseSQLiteValue>>,
}

impl<'connection> RiseSQLiteStatement<'connection> {
    fn new(
        pointer: RiseSQLiteStatementPointer<'connection>,
        operation: &str,
        owner: &'connection RiseSQLiteConnection,
        cache_key: &str,
        returns_to_cache: bool,
    ) -> Self {
        Self {
            pointer: Some(pointer),
            operation: operation.to_owned(),
            owner,
            cache_key: cache_key.to_owned(),
            returns_to_cache,
            pending_rows: None,
            current_row: None,
        }
    }

    pub(crate) fn release(&mut self) {
        let Some(pointer) = self.pointer.take() else {
            return;
        };
        self.pending_rows = None;
        self.current_row = None;
        drop(pointer);
        self.owner
            .return_statement(&self.cache_key, self.returns_to_cache);
    }

    pub(crate) fn bind(&mut self, value: impl ToSql, at: i32) -> Result<(), RiseStoreError> {
        let suffix = rise_sq_lite_bind_suffix(&value);
        self.bind_value(value, at, suffix)
    }

    pub(crate) fn bind_null(&mut self, at: i32) -> Result<(), RiseStoreError> {
        self.bind_value(Null, at, "bind_null")
    }

    pub(crate) fn step(&mut self) -> Result<bool, RiseStoreError> {
        if self.pending_rows.is_none() {
            let Some(pointer) = self.pointer.as_mut() else {
                return Err(RiseStoreError::NotOpen);
            };
            let rows = rise_sq_lite_collect_rows(pointer.statement()).map_err(|error| {
                rise_sq_lite_error(&error, &format!("{}.step", self.operation))
            })?;
            self.pending_rows = Some(rows);
        }
        let next = self
            .pending_rows
            .as_mut()
            .and_then(|rows| rows.pop_front());
        let has_row = next.is_some();
        self.current_row = next;
        Ok(has_row)
    }

    pub(crate) fn text(&self, at: i32) -> Option<String> {
        let bytes = self.column(at)?.text_bytes()?;
        String::from_utf8(bytes).ok()
    }

    pub(crate) fn int64(&self, at: i32) -> i64 {
        self.column(at).map_or(0, RiseSQLiteValue::int64)
    }

    pub(crate) fn bool(&self, at: i32) -> bool {
        self.int64(at) != 0
    }

    pub(crate) fn is_null(&self, at: i32) -> bool {
        self.column(at)
            .is_none_or(|value| *value == RiseSQLiteValue::Null)
    }

    pub(crate) fn byte_count(&self, at: i32) -> i64 {
        self.column(at)
            .map_or(0, |value| i64::try_from(value.byte_count()).unwrap_or(i64::MAX))
    }

    pub(crate) fn data(&self, at: i32) -> Option<Vec<u8>> {
        self.column(at)?.text_bytes()
    }

    fn column(&self, at: i32) -> Option<&RiseSQLiteValue> {
        let index = usize::try_from(at).ok()?;
        self.current_row.as_ref()?.get(index)
    }

    fn bind_value(
        &mut self,
        value: impl ToSql,
        at: i32,
        suffix: &str,
    ) -> Result<(), RiseStoreError> {
        let Some(pointer) = self.pointer.as_mut() else {
            return Err(RiseStoreError::NotOpen);
        };
        let Ok(index) = usize::try_from(at) else {
            return Err(RiseStoreError::Sqlite {
                code: rusqlite::ffi::SQLITE_RANGE,
                operation: format!("{}.{suffix}", self.operation),
            });
        };
        pointer
            .statement()
            .raw_bind_parameter(index, value)
            .map_err(|error| rise_sq_lite_error(&error, &format!("{}.{suffix}", self.operation)))
    }
}

impl Drop for RiseSQLiteStatement<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

fn rise_sq_lite_collect_rows(
    statement: &mut Statement<'_>,
) -> Result<VecDeque<Vec<RiseSQLiteValue>>, rusqlite::Error> {
    let column_count = statement.column_count();
    let mut collected = VecDeque::new();
    let mut rows = statement.raw_query();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(RiseSQLiteValue::from_value_ref(row.get_ref(index)?));
        }
        collected.push_back(values);
    }
    Ok(collected)
}

fn rise_sq_lite_bind_suffix(value: &impl ToSql) -> &'static str {
    match value.to_sql() {
        Ok(ToSqlOutput::Borrowed(ValueRef::Null) | ToSqlOutput::Owned(Value::Null)) => "bind_null",
        Ok(ToSqlOutput::Borrowed(ValueRef::Integer(_)) | ToSqlOutput::Owned(Value::Integer(_))) => {
            "bind_int64"
        }
        Ok(ToSqlOutput::Borrowed(ValueRef::Real(_)) | ToSqlOutput::Owned(Value::Real(_))) => {
            "bind_double"
        }
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(_)) | ToSqlOutput::Owned(Value::Text(_))) => {
            "bind_text"
        }
        _ => "bind_blob",
    }
}

fn rise_sq_lite_leading_integer(bytes: &[u8]) -> i64 {
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };
    let mut value: i64 = 0;
    while let Some(digit) = bytes.get(index).filter(|byte| byte.is_ascii_digit()) {
        let digit = i64::from(digit - b'0');
        value = if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        };
        index += 1;
    }
    value
}

fn rise_sq_lite_error(error: &rusqlite::Error, operation: &str) -> RiseStoreError {
    let code = match error {
        rusqlite::Error::SqliteFailure(failure, _) => failure.extended_code & 0xff,
        _ => rusqlite::ffi::SQLITE_MISUSE,
    };
    RiseStoreError::Sqlite {
        code,
        operation: operation.to_owned(),
    }
}
