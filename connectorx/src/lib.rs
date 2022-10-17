#![allow(incomplete_features)]
#![allow(clippy::upper_case_acronyms)]
#![feature(vec_into_raw_parts)]

//! # ConnectorX
//!
//! ConnectorX enables you to load data from databases into dataframes in the fastest and most memory efficient way by leveraging
//! zero-copy and partition-based parallelism.
//!
//! Currently, ConnectorX consists of a Rust core library and a python library. This is the documentation for the Rust crate.
//! For the documentation of the Python library, please refer to our [Github Readme](https://github.com/sfu-db/connector-x).
//!
//! # Design
//!
//! A data loading problem consists of three sub-problems:
//! 1. How to connect to the data source and read data.
//! 2. How to connect to the data destination and write data.
//! 3. How to map the types between the source and destination.
//!
//! Additionally, since ConnectorX will partition a query into partitions and execute them in parallel, we also have
//! 4. How to partition the query and run them in parallel.
//!
//! ConnectorX approaches these problems by defining abstractions on sources, destinations, and mapping rules.
//! For the partition-based parallelism, ConnectorX will partition the query as well as the source and the destination
//! together and put them into threads.
//! Each thread will own exactly 1 query, 1 partitioned source, and 1 partitioned destination.
//!
//! The following graph depicts the internal mechanism when ConnectorX is downloading the data.
//!
//! ```text
//!                     +------------------------------------------------------------+
//!                     |                           Thread 1                         |
//!                     |                                                            |
//!     +---+           | +-----------------+   +-------------+  +-----------------+ |          +---+
//!     |   +-----------+>| Partitioned Src +-->| Type Mapper +->| Partitioned Dst +-+--------->|   |
//!     |   |           | +-----------------+   +-------------+  +-----------------+ |          |   |
//!     | D |           |                                                            |          | D |
//!     | a |           +------------------------------------------------------------+          | a |
//!     | t |                                          .                                        | t |
//!     | a |                                          .                                        | a |
//!     | b |                                          .                                        | f |
//!     | a |           +------------------------------------------------------------+          | r |
//!     | s |           |                           Thread n                         |          | a |
//!     | e |           |                                                            |          | m |
//!     |   |           | +-----------------+   +-------------+  +-----------------+ |          | e |
//!     |   +-----------+>| Partitioned Src +-->| Type Mapper +->| Partitioned Dst +-+--------->|   |
//!     +---+           | +-----------------+   +-------------+  +-----------------+ |          +---+
//!                     |                                                            |
//!                     +------------------------------------------------------------+
//!
//! ```
//! ## How does ConnectorX download the data?
//!
//! Upon receiving the query, e.g. SELECT * FROM lineitem, ConnectorX will first issue a LIMIT 1 query SELECT * FROM lineitem LIMIT 1 to get the schema of the result set.
//!
//! Then, if partition_on is specified, ConnectorX will issue `SELECT MIN($partition_on), MAX($partition_on) FROM (SELECT * FROM lineitem)` to know the range of the partition column.
//! After that, the original query is split into partitions based on the min/max information, e.g. `SELECT * FROM (SELECT * FROM lineitem) WHERE $partition_on > 0 AND $partition_on < 10000`.
//! ConnectorX will then run a count query to get the partition size (e.g. `SELECT COUNT(*) FROM (SELECT * FROM lineitem) WHERE $partition_on > 0 AND $partition_on < 10000`).
//! If the partition is not specified, the count query will be `SELECT COUNT(*) FROM (SELECT * FROM lineitem)`.
//!
//! Finally, ConnectorX will use the schema info as well as the count info to allocate memory and download data by executing the queries normally.
//! Once the downloading begins, there will be one thread for each partition so that the data are downloaded in parallel at the partition level.
//! The thread will issue the query of the corresponding partition to the database and then write the returned data to the destination row-wise or column-wise (depends on the database) in a streaming fashion.
//! This mechanism implies that having an index on the partition column is recommended to make full use of the parallel downloading power provided by ConnectorX.
//!
//! # Extending ConnectorX
//! ## Adding a new source
//!
//! To add a new data source, you need to implement [`sources::Source`], [`sources::SourcePartition`], [`sources::PartitionParser`], and [`sources::Produce`] for the source.
//! In detail, [`sources::Source`] describes how to connect to the database from a connection string, as well as how to do partitioning on the source to produce a list of [`sources::SourcePartition`].
//! [`sources::SourcePartition`] describes how to get the row count for the specific partition so that the destination can preallocate the memory.
//! Finally, [`sources::PartitionParser`] and [`sources::Produce`] abstracts away the detail about how does each partition parse different types.
//!
//! ## Adding a new destination
//!
//! To add a new data destination, you need to implement [`destinations::Destination`], [`destinations::DestinationPartition`], and [`destinations::Consume`]. Similar to the sources,
//! [`destinations::Destination`] describes how to allocate the memory of the data destination, as well as how to do partitioning on the destination to produce a list of [`destinations::DestinationPartition`].
//! [`destinations::DestinationPartition`] and [`destinations::Consume`] abstract away the detail about how does each partition writes different types.
//!
//! ## Adding a new transport (type mapping)
//!
//! After having a source and a destination that describes how to read and write the data,
//! ConnectorX also needs to know how to convert the values with different types from the source to the destination.
//! For example, Postgres can produce a `uuid` type but there's no uuid in Arrow. It is the transport's duty to convert
//! the `uuid` into an Arrow compatible type, e.g. string. You can use the [`impl_transport!`] macro to define a transport.
//!
//! ## Putting things together
//!
//! Say, you decide to load data from SQL Server to Arrow. In ConnectorX we already provided the source for SQL Server as [`sources::sqlite::SQLiteSource`], and the
//! Arrow destination [`destinations::arrow::ArrowDestination`], as well as the transport [`transports::SQLiteArrowTransport`].
//! Given the source, destination and transport already implemented, you can use [`dispatcher::Dispatcher`] to load the data:
//!
//! ```no_run
//! use connectorx::prelude::*;
//!
//! let mut destination = ArrowDestination::new();
//! let source = SQLiteSource::new("/path/to/db", 10).expect("cannot create the source");
//! let queries = &["SELECT * FROM db WHERE id < 100", "SELECT * FROM db WHERE id >= 100"];
//! let dispatcher = Dispatcher::<SQLiteSource, ArrowDestination, SQLiteArrowTransport>::new(source, &mut destination, queries, None);
//! dispatcher.run().expect("run failed");
//!
//! let data = destination.arrow();
//! ```
//!
//! Or simply you can directly use the [`get_arrow::get_arrow`] or [`get_arrow2::get_arrow2`] in which we wrapped the above procedures:
//!
//! ```no_run
//! use connectorx::prelude::*;
//!
//! let mut source_conn = SourceConn::try_from("postgresql://username:password@host:port/db?cxprotocol=binary");
//! let queries = &["SELECT * FROM table WHERE id < 100", "SELECT * FROM table WHERE id >= 100"];
//! let destination = get_arrow(source_conn, Some("SELECT * FROM table"), queries).expect("run failed");
//!
//! let data = destination.arrow();
//! ```
//!
//! NOTE: the pool size parameter `nconn` used in initializing the source should be larger than or equal to the number of partitioned queries input later.
//!
//! ## Need more examples?
//! You can use the existing implementation as the example.
//! [MySQL source](https://github.com/sfu-db/connector-x/tree/main/connectorx/src/sources/mysql),
//! [Arrow destination](https://github.com/sfu-db/connector-x/tree/main/connectorx/src/destinations/arrow),
//! [MySQL to Arrow transport](https://github.com/sfu-db/connector-x/blob/main/connectorx/src/transports/mysql_arrow.rs).
//!
//! # Sources protocols & Destinations that is implemented in the Rust core.
//!
//! ## Sources
//! - [x] Postgres
//! - [x] Mysql
//! - [x] Sqlite
//! - [x] SQL Server
//! - [x] Oracle
//! - [x] BigQuery
//!
//! ## Destinations
//! - [x] Arrow
//! - [x] Arrow2
//!
//! # Feature gates
//! By default, ConnectorX does not enable any sources / destinations to keep the dependencies minimal.
//! Instead, we provide following features for you to opt-in: `src_sqlite`, `src_postgres`, `src_mysql`, `src_mssql`, `src_oracle`, `dst_arrow`, `dst_arrow2`.
//! For example, if you'd like to load data from Postgres to Arrow, you can enable `src_postgres` and `dst_arrow` in `Cargo.toml`.
//! This will enable [`sources::postgres`], [`destinations::arrow`] and [`transports::PostgresArrowTransport`].

use libc::c_char;
use source_router::SourceConn;
use sql::CXQuery;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::env;
use std::ffi::{CStr, CString};

pub mod typesystem;
#[macro_use]
mod macros;
pub mod constants;
pub mod data_order;
pub mod destinations;
mod dispatcher;
pub mod errors;
#[cfg(feature = "federation")]
pub mod fed_dispatcher;
#[cfg(feature = "dst_arrow")]
pub mod get_arrow;
#[cfg(feature = "dst_arrow2")]
pub mod get_arrow2;
pub mod source_router;
pub mod sources;
#[doc(hidden)]
pub mod sql;
pub mod transports;
#[doc(hidden)]
pub mod utils;

pub mod prelude {
    pub use crate::data_order::{coordinate, DataOrder};
    #[cfg(feature = "dst_arrow")]
    pub use crate::destinations::arrow::ArrowDestination;
    #[cfg(feature = "dst_arrow2")]
    pub use crate::destinations::arrow2::Arrow2Destination;
    pub use crate::destinations::{Consume, Destination, DestinationPartition};
    pub use crate::dispatcher::Dispatcher;
    pub use crate::errors::{ConnectorXError, ConnectorXOutError};
    #[cfg(feature = "dst_arrow")]
    pub use crate::get_arrow::get_arrow;
    #[cfg(feature = "dst_arrow2")]
    pub use crate::get_arrow2::get_arrow2;
    pub use crate::source_router::*;
    #[cfg(feature = "src_bigquery")]
    pub use crate::sources::bigquery::BigQuerySource;
    #[cfg(feature = "src_csv")]
    pub use crate::sources::csv::CSVSource;
    #[cfg(feature = "src_dummy")]
    pub use crate::sources::dummy::DummySource;
    #[cfg(feature = "src_mssql")]
    pub use crate::sources::mssql::MsSQLSource;
    #[cfg(feature = "src_mysql")]
    pub use crate::sources::mysql::MySQLSource;
    #[cfg(feature = "src_oracle")]
    pub use crate::sources::oracle::OracleSource;
    #[cfg(feature = "src_postgres")]
    pub use crate::sources::postgres::PostgresSource;
    #[cfg(feature = "src_sqlite")]
    pub use crate::sources::sqlite::SQLiteSource;
    pub use crate::sources::{PartitionParser, Produce, Source, SourcePartition};
    pub use crate::transports::*;
    pub use crate::typesystem::{
        ParameterizedFunc, ParameterizedOn, Realize, Transport, TypeAssoc, TypeConversion,
        TypeSystem,
    };
}

#[repr(C)]
pub struct CXSlice<T> {
    ptr: *const T,
    len: usize,
    capacity: usize,
}

impl<T> CXSlice<T> {
    pub fn new_from_vec(v: Vec<T>) -> Self {
        let (ptr, len, capacity) = v.into_raw_parts();
        Self { ptr, len, capacity }
    }
}

#[repr(C)]
pub struct CXTable {
    name: *const c_char,
    columns: CXSlice<*const c_char>,
}

#[repr(C)]
pub struct CXConnectionInfo {
    name: *const c_char,
    conn: *const c_char,
    schema: CXSlice<CXTable>,
    is_local: bool,
}

#[repr(C)]
pub struct CXFederatedPlan {
    db_name: *const c_char,
    db_alias: *const c_char,
    sql: *const c_char,
}

#[cfg(feature = "federation")]
#[no_mangle]
pub unsafe extern "C" fn free_plans(res: *const CXSlice<CXFederatedPlan>) {
    let plans = get_vec::<_>((*res).ptr, (*res).len, (*res).capacity);
    plans.into_iter().for_each(|plan| {
        free_str(plan.db_name);
        free_str(plan.db_alias);
        free_str(plan.sql);
    });
}

#[cfg(feature = "federation")]
#[no_mangle]
pub unsafe extern "C" fn connectorx_rewrite(
    conn_list: *const CXSlice<CXConnectionInfo>,
    query: *const c_char,
) -> CXSlice<CXFederatedPlan> {
    use crate::fed_dispatcher::FederatedDataSourceInfo;

    let mut db_map = HashMap::new();
    let conn_slice = unsafe { std::slice::from_raw_parts((*conn_list).ptr, (*conn_list).len) };
    for p in conn_slice {
        let name = unsafe { CStr::from_ptr(p.name) }.to_str().unwrap();
        if p.conn.is_null() {
            let mut table_map: HashMap<String, Vec<String>> = HashMap::new();
            let table_slice = unsafe { std::slice::from_raw_parts(p.schema.ptr, p.schema.len) };
            for t in table_slice {
                let table_name = unsafe { CStr::from_ptr(t.name) }.to_str().unwrap();
                println!("raw table name: {:?}", table_name);
                let column_slice =
                    unsafe { std::slice::from_raw_parts(t.columns.ptr, t.columns.len) };

                let mut column_names = vec![];
                for &c in column_slice {
                    let column_name = unsafe { CStr::from_ptr(c).to_str().unwrap() };
                    column_names.push(column_name.to_string());
                }
                table_map.insert(table_name.to_string(), column_names);
            }
            let source_info =
                FederatedDataSourceInfo::new_from_manual_schema(table_map, p.is_local);
            db_map.insert(name.to_string(), source_info);
        } else {
            let conn = unsafe { CStr::from_ptr(p.conn) }.to_str().unwrap();
            println!("name: {:?}, conn: {:?}", name, conn);
            let source_info = FederatedDataSourceInfo::new_from_conn_str(
                SourceConn::try_from(conn).unwrap(),
                p.is_local,
            );
            db_map.insert(name.to_string(), source_info);
        }
    }

    let query_str = unsafe { CStr::from_ptr(query) }.to_str().unwrap();
    let j4rs_base = match env::var("CX_LIB_PATH") {
        Ok(val) => Some(val),
        Err(_) => None,
    };
    println!("j4rs_base: {:?}", j4rs_base);
    let fed_plan: Vec<CXFederatedPlan> =
        fed_dispatcher::rewrite_sql(query_str, &db_map, j4rs_base.as_deref())
            .unwrap()
            .into_iter()
            .map(|p| p.into())
            .collect();

    CXSlice::<_>::new_from_vec(fed_plan)
}

#[repr(C)]
pub struct CXArray {
    array: *const arrow::ffi::FFI_ArrowArray,
    schema: *const arrow::ffi::FFI_ArrowSchema,
}

#[repr(C)]
pub struct CXResult {
    data: CXSlice<CXSlice<CXArray>>,
    header: CXSlice<*const c_char>,
}

pub unsafe fn get_vec<T>(ptr: *const T, len: usize, capacity: usize) -> Vec<T> {
    Vec::from_raw_parts(ptr as *mut T, len, capacity)
}

pub unsafe fn free_str(ptr: *const c_char) {
    let _ = CString::from_raw(ptr as *mut _);
}

#[no_mangle]
pub unsafe extern "C" fn free_result(res: *const CXResult) {
    let header = get_vec::<_>((*res).header.ptr, (*res).header.len, (*res).header.capacity);
    header.into_iter().for_each(|col| free_str(col));

    let rbs = get_vec::<_>((*res).data.ptr, (*res).data.len, (*res).data.capacity);
    rbs.into_iter().for_each(|rb| {
        get_vec::<_>(rb.ptr, rb.len, rb.capacity)
            .into_iter()
            .for_each(|a| {
                // Otherwise memory leak
                std::sync::Arc::from_raw(a.array);
                std::sync::Arc::from_raw(a.schema);
            })
    });
}

#[cfg(feature = "dst_arrow")]
#[no_mangle]
pub extern "C" fn connectorx_scan(conn: *const i8, query: *const i8) -> CXResult {
    let conn_str = unsafe { CStr::from_ptr(conn) }.to_str().unwrap();
    let query_str = unsafe { CStr::from_ptr(query) }.to_str().unwrap();
    let source_conn = SourceConn::try_from(conn_str).unwrap();
    let record_batches = get_arrow::get_arrow(&source_conn, None, &[CXQuery::from(query_str)])
        .unwrap()
        .arrow()
        .unwrap();

    // arrow::util::pretty::print_batches(&record_batches[..]).unwrap();

    let names: Vec<*const c_char> = record_batches[0]
        .schema()
        .fields()
        .iter()
        .map(|f| {
            CString::new(f.name().as_str())
                .expect("new CString error")
                .into_raw() as *const c_char
        })
        .collect();

    let mut result = vec![];
    for rb in record_batches {
        let mut cols = vec![];

        for array in rb.columns() {
            let data = array.data().clone();
            let array = arrow::ffi::ArrowArray::try_new(data).expect("c ptr");
            let (array_ptr, schema_ptr) = arrow::ffi::ArrowArray::into_raw(array);

            let cx_array = CXArray {
                array: array_ptr,
                schema: schema_ptr,
            };
            cols.push(cx_array);
        }

        let cx_rb = CXSlice::<CXArray>::new_from_vec(cols);
        result.push(cx_rb);
    }

    let res = CXResult {
        data: CXSlice::<_>::new_from_vec(result),
        header: CXSlice::<_>::new_from_vec(names),
    };

    res
}
