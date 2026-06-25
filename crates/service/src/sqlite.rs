use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;

use libloading::Library;

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_NULL: c_int = 5;
const SQLITE_OPEN_READONLY: c_int = 0x0000_0001;
const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;
const SQLITE_OPEN_NOMUTEX: c_int = 0x0000_8000;

type Sqlite3 = c_void;
type Sqlite3Stmt = c_void;
type Destructor = Option<unsafe extern "C" fn(*mut c_void)>;
type InitFn = unsafe extern "C" fn() -> c_int;
type OpenFn = unsafe extern "C" fn(*const c_char, *mut *mut Sqlite3, c_int, *const c_char) -> c_int;
type CloseFn = unsafe extern "C" fn(*mut Sqlite3) -> c_int;
type ErrmsgFn = unsafe extern "C" fn(*mut Sqlite3) -> *const c_char;
type ExecFn = unsafe extern "C" fn(
    *mut Sqlite3,
    *const c_char,
    *mut c_void,
    *mut c_void,
    *mut *mut c_char,
) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type PrepareFn = unsafe extern "C" fn(
    *mut Sqlite3,
    *const c_char,
    c_int,
    *mut *mut Sqlite3Stmt,
    *mut *const c_char,
) -> c_int;
type StmtFn = unsafe extern "C" fn(*mut Sqlite3Stmt) -> c_int;
type BindNullFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> c_int;
type BindIntFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, i64) -> c_int;
type BindDoubleFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, f64) -> c_int;
type BindTextFn =
    unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, *const c_char, c_int, Destructor) -> c_int;
type BindBlobFn =
    unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, *const c_void, c_int, Destructor) -> c_int;
type ColumnTypeFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> c_int;
type ColumnIntFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> i64;
type ColumnDoubleFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> f64;
type ColumnTextFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> *const u8;
type ColumnBlobFn = unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> *const c_void;
type LastInsertFn = unsafe extern "C" fn(*mut Sqlite3) -> i64;

#[derive(Debug)]
pub struct SqliteError(String);

impl SqliteError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SqliteError {}

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::Integer(value as i64)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

struct Api {
    _library: Library,
    initialize: InitFn,
    open_v2: OpenFn,
    close: CloseFn,
    errmsg: ErrmsgFn,
    exec: ExecFn,
    free: FreeFn,
    prepare_v2: PrepareFn,
    finalize: StmtFn,
    step: StmtFn,
    reset: StmtFn,
    clear_bindings: StmtFn,
    bind_null: BindNullFn,
    bind_int64: BindIntFn,
    bind_double: BindDoubleFn,
    bind_text: BindTextFn,
    bind_blob: BindBlobFn,
    column_type: ColumnTypeFn,
    column_int64: ColumnIntFn,
    column_double: ColumnDoubleFn,
    column_text: ColumnTextFn,
    column_blob: ColumnBlobFn,
    column_bytes: ColumnTypeFn,
    last_insert_rowid: LastInsertFn,
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

fn api() -> Result<&'static Api, SqliteError> {
    API.get_or_init(load_api)
        .as_ref()
        .map_err(|message| SqliteError::new(message.clone()))
}

fn load_api() -> Result<Api, String> {
    let mut errors = Vec::new();
    for candidate in sqlite_library_candidates() {
        let library = match unsafe { Library::new(&candidate) } {
            Ok(library) => library,
            Err(error) => {
                errors.push(format!("{}: {error}", candidate.display()));
                continue;
            }
        };
        unsafe {
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|error| format!("{}: {error}", $name))?
                };
            }
            let api = Api {
                initialize: symbol!("sqlite3_initialize", InitFn),
                open_v2: symbol!("sqlite3_open_v2", OpenFn),
                close: symbol!("sqlite3_close", CloseFn),
                errmsg: symbol!("sqlite3_errmsg", ErrmsgFn),
                exec: symbol!("sqlite3_exec", ExecFn),
                free: symbol!("sqlite3_free", FreeFn),
                prepare_v2: symbol!("sqlite3_prepare_v2", PrepareFn),
                finalize: symbol!("sqlite3_finalize", StmtFn),
                step: symbol!("sqlite3_step", StmtFn),
                reset: symbol!("sqlite3_reset", StmtFn),
                clear_bindings: symbol!("sqlite3_clear_bindings", StmtFn),
                bind_null: symbol!("sqlite3_bind_null", BindNullFn),
                bind_int64: symbol!("sqlite3_bind_int64", BindIntFn),
                bind_double: symbol!("sqlite3_bind_double", BindDoubleFn),
                bind_text: symbol!("sqlite3_bind_text", BindTextFn),
                bind_blob: symbol!("sqlite3_bind_blob", BindBlobFn),
                column_type: symbol!("sqlite3_column_type", ColumnTypeFn),
                column_int64: symbol!("sqlite3_column_int64", ColumnIntFn),
                column_double: symbol!("sqlite3_column_double", ColumnDoubleFn),
                column_text: symbol!("sqlite3_column_text", ColumnTextFn),
                column_blob: symbol!("sqlite3_column_blob", ColumnBlobFn),
                column_bytes: symbol!("sqlite3_column_bytes", ColumnTypeFn),
                last_insert_rowid: symbol!("sqlite3_last_insert_rowid", LastInsertFn),
                _library: library,
            };
            let code = (api.initialize)();
            if code != SQLITE_OK {
                return Err(format!("sqlite3_initialize failed with code {code}"));
            }
            return Ok(api);
        }
    }
    Err(format!(
        "Could not load SQLite library. Set PHS_SQLITE3_LIB. Attempts: {}",
        errors.join("; ")
    ))
}

fn sqlite_library_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = env::var_os("PHS_SQLITE3_LIB") {
        paths.push(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from("sqlite3.dll"));
        if let Some(home) = env::var_os("USERPROFILE") {
            paths.push(PathBuf::from(home).join(
                ".cache/codex-runtimes/codex-primary-runtime/dependencies/python/DLLs/sqlite3.dll",
            ));
        }
    }
    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("libsqlite3.so.0"));
        paths.push(PathBuf::from("libsqlite3.so"));
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("libsqlite3.dylib"));
    }
    paths
}

pub struct Connection {
    api: &'static Api,
    raw: *mut Sqlite3,
}

impl Connection {
    pub fn open(path: &Path, read_only: bool) -> Result<Self, SqliteError> {
        let api = api()?;
        let path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| SqliteError::new("SQLite path contains NUL"))?;
        let flags = if read_only {
            SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX
        } else {
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOMUTEX
        };
        let mut raw = ptr::null_mut();
        let code = unsafe { (api.open_v2)(path.as_ptr(), &mut raw, flags, ptr::null()) };
        if code != SQLITE_OK {
            let message = if raw.is_null() {
                format!("sqlite3_open_v2 failed with code {code}")
            } else {
                unsafe { CStr::from_ptr((api.errmsg)(raw)) }
                    .to_string_lossy()
                    .into_owned()
            };
            if !raw.is_null() {
                unsafe { (api.close)(raw) };
            }
            return Err(SqliteError::new(message));
        }
        Ok(Self { api, raw })
    }

    pub fn exec(&self, sql: &str) -> Result<(), SqliteError> {
        let sql = CString::new(sql).map_err(|_| SqliteError::new("SQL contains NUL"))?;
        let mut error_message = ptr::null_mut();
        let code = unsafe {
            (self.api.exec)(
                self.raw,
                sql.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut error_message,
            )
        };
        if code == SQLITE_OK {
            return Ok(());
        }
        let message = if error_message.is_null() {
            self.error_message()
        } else {
            let message = unsafe { CStr::from_ptr(error_message) }
                .to_string_lossy()
                .into_owned();
            unsafe { (self.api.free)(error_message.cast()) };
            message
        };
        Err(SqliteError::new(message))
    }

    pub fn prepare<'a>(&'a self, sql: &str) -> Result<Statement<'a>, SqliteError> {
        let sql = CString::new(sql).map_err(|_| SqliteError::new("SQL contains NUL"))?;
        let mut raw = ptr::null_mut();
        let code =
            unsafe { (self.api.prepare_v2)(self.raw, sql.as_ptr(), -1, &mut raw, ptr::null_mut()) };
        self.check(code)?;
        Ok(Statement {
            connection: self,
            raw,
        })
    }

    pub fn execute(&self, sql: &str, values: &[Value]) -> Result<(), SqliteError> {
        let mut statement = self.prepare(sql)?;
        statement.execute(values)
    }

    pub fn last_insert_rowid(&self) -> i64 {
        unsafe { (self.api.last_insert_rowid)(self.raw) }
    }

    fn check(&self, code: c_int) -> Result<(), SqliteError> {
        if code == SQLITE_OK {
            Ok(())
        } else {
            Err(SqliteError::new(self.error_message()))
        }
    }

    fn error_message(&self) -> String {
        unsafe { CStr::from_ptr((self.api.errmsg)(self.raw)) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            (self.api.close)(self.raw);
        }
    }
}

pub struct Statement<'a> {
    connection: &'a Connection,
    raw: *mut Sqlite3Stmt,
}

impl Statement<'_> {
    pub fn start(&mut self, values: &[Value]) -> Result<(), SqliteError> {
        self.connection
            .check(unsafe { (self.connection.api.reset)(self.raw) })?;
        self.connection
            .check(unsafe { (self.connection.api.clear_bindings)(self.raw) })?;
        for (index, value) in values.iter().enumerate() {
            self.bind((index + 1) as c_int, value)?;
        }
        Ok(())
    }

    pub fn step_row(&mut self) -> Result<bool, SqliteError> {
        match unsafe { (self.connection.api.step)(self.raw) } {
            SQLITE_ROW => Ok(true),
            SQLITE_DONE => Ok(false),
            _ => Err(SqliteError::new(self.connection.error_message())),
        }
    }

    pub fn execute(&mut self, values: &[Value]) -> Result<(), SqliteError> {
        self.start(values)?;
        if self.step_row()? {
            return Err(SqliteError::new(
                "SQLite execute unexpectedly returned a row",
            ));
        }
        Ok(())
    }

    pub fn column_i64(&self, index: c_int) -> i64 {
        unsafe { (self.connection.api.column_int64)(self.raw, index) }
    }

    pub fn column_u32(&self, index: c_int) -> Result<u32, SqliteError> {
        u32::try_from(self.column_i64(index))
            .map_err(|_| SqliteError::new(format!("Column {index} is outside u32 range")))
    }

    pub fn column_f64(&self, index: c_int) -> f64 {
        unsafe { (self.connection.api.column_double)(self.raw, index) }
    }

    pub fn column_text(&self, index: c_int) -> Result<String, SqliteError> {
        if self.column_type(index) == SQLITE_NULL {
            return Err(SqliteError::new(format!("Column {index} is NULL")));
        }
        let pointer = unsafe { (self.connection.api.column_text)(self.raw, index) };
        let length = unsafe { (self.connection.api.column_bytes)(self.raw, index) };
        if pointer.is_null() {
            return Ok(String::new());
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer, length as usize) };
        String::from_utf8(bytes.to_vec()).map_err(|error| SqliteError::new(error.to_string()))
    }

    pub fn column_blob(&self, index: c_int) -> Vec<u8> {
        let pointer = unsafe { (self.connection.api.column_blob)(self.raw, index) };
        let length = unsafe { (self.connection.api.column_bytes)(self.raw, index) };
        if pointer.is_null() || length <= 0 {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length as usize) }.to_vec()
    }

    pub fn column_optional_f64(&self, index: c_int) -> Option<f64> {
        (self.column_type(index) != SQLITE_NULL).then(|| self.column_f64(index))
    }

    fn column_type(&self, index: c_int) -> c_int {
        unsafe { (self.connection.api.column_type)(self.raw, index) }
    }

    fn bind(&self, index: c_int, value: &Value) -> Result<(), SqliteError> {
        let transient: Destructor = unsafe { std::mem::transmute(-1isize) };
        let code = unsafe {
            match value {
                Value::Null => (self.connection.api.bind_null)(self.raw, index),
                Value::Integer(value) => (self.connection.api.bind_int64)(self.raw, index, *value),
                Value::Real(value) => (self.connection.api.bind_double)(self.raw, index, *value),
                Value::Text(value) => (self.connection.api.bind_text)(
                    self.raw,
                    index,
                    value.as_ptr().cast(),
                    value.len() as c_int,
                    transient,
                ),
                Value::Blob(value) => (self.connection.api.bind_blob)(
                    self.raw,
                    index,
                    value.as_ptr().cast(),
                    value.len() as c_int,
                    transient,
                ),
            }
        };
        self.connection.check(code)
    }
}

impl Drop for Statement<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.connection.api.finalize)(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_memory_database() {
        let connection = Connection::open(Path::new(":memory:"), false).unwrap();
        connection
            .exec("CREATE TABLE test(id INTEGER PRIMARY KEY, value TEXT)")
            .unwrap();
        connection
            .execute("INSERT INTO test(value) VALUES (?1)", &[Value::from("ok")])
            .unwrap();
        let mut statement = connection.prepare("SELECT id, value FROM test").unwrap();
        statement.start(&[]).unwrap();
        assert!(statement.step_row().unwrap());
        assert_eq!(statement.column_i64(0), 1);
        assert_eq!(statement.column_text(1).unwrap(), "ok");
        assert!(!statement.step_row().unwrap());
    }
}
