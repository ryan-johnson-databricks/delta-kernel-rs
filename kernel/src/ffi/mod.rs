/// Contains code the exposes what an engine needs to call from 'c' to interface with kernel
use arrow_array::RecordBatch;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use url::Url;

use crate::{ArrowSchemaRef, DeltaResult, ExpressionEvaluator, ExpressionHandler, TableClient};
use crate::expressions::{BinaryOperator, Expression, scalars::Scalar};
use crate::scan::ScanBuilder;
use crate::schema::Schema;

/*
 Note: Right now the iterator simply returns void*s, but we could rather have a generic type on
 EngineIterator and have more type safety. This would be at the cost of having to have a
 `create_[type]_iterator` function for each type we want to support, as 'extern' functions can't be
 generic, so we can't write: `extern fn create_iterator<T> -> EngineIterator<T>`, and have to rather
 do: `extern fn create_int_iterator -> EngineIterator<i32>`
 */

#[repr(C)]
pub struct EngineExpressionEvaluator {
    data: *mut c_void,
    evaluate_fn: extern fn(data: *mut c_void, batch: *const RecordBatch) -> DeltaResult<RecordBatch>,
}

#[repr(C)]
pub struct EngineExpressionHandler {
    data: *mut c_void,
    get_evaluator_fn: extern fn(
        data: *mut c_void,
        schema: *const Schema,
        expression: *const Expression) -> *mut EngineExpressionEvaluator,
    drop_evaluator_fn: extern fn(data: *mut c_void, evaluator: *mut EngineExpressionEvaluator) -> (),
}

#[repr(C)]
pub struct EngineFileSystemClient {}

#[repr(C)]
pub struct EngineJsonHandler {}

#[repr(C)]
pub struct EngineParquetHandler {}

#[repr(C)]
pub struct EngineTableClient {
    data: *mut c_void,

    // These can be used to obtain a reference to the engine's various handlers. Each reference
    // obtained by a _get must be returned to the engine by a call to the corresponding _put.
    get_expression_handler_fn: extern fn(data: *mut c_void) -> *mut EngineExpressionHandler,
    get_file_system_client_fn: extern fn(data: *mut c_void) -> *mut EngineFileSystemClient,
    get_json_handler_fn: extern fn(data: *mut c_void) -> *mut EngineJsonHandler,
    get_parquet_handler_fn: extern fn(data: *mut c_void) -> *mut EngineParquetHandler,

    drop_expression_handler_fn: extern fn(data: *mut c_void, ptr: *mut EngineExpressionHandler) -> (),
    drop_file_system_client_fn: extern fn(data: *mut c_void, ptr: *mut EngineFileSystemClient) -> (),
    drop_json_handler_fn: extern fn(data: *mut c_void, ptr: *mut EngineJsonHandler) -> (),
    drop_parquet_handler_fn: extern fn(data: *mut c_void, ptr: *mut EngineParquetHandler) -> (),
}

struct EngineTraitWrapper<T> {
    data: *mut c_void,
    drop_fn: extern fn(data: *mut c_void, ptr: *mut T) -> (),
    ptr: *mut T,
}

impl ExpressionEvaluator for EngineTraitWrapper<EngineExpressionEvaluator> {
    /// Evaluate the expression on given ColumnarBatch data.
    ///
    /// Contains one value for each row of the input.
    /// The data type of the output is same as the type output of the expression this evaluator is using.
    fn evaluate(&self, batch: &RecordBatch) -> DeltaResult<RecordBatch> {
        todo!()
    }
}

impl ExpressionHandler for EngineTraitWrapper<EngineExpressionHandler> {
    fn get_evaluator(
        &self,
        schema: ArrowSchemaRef,
        expression: Expression,
    ) -> Arc<dyn ExpressionEvaluator> {
        let handler = unsafe { &*self.ptr };
        let schema = Schema::try_from(schema).unwrap();
        let wrapper = EngineTraitWrapper {
            data: self.data,
            drop_fn: handler.drop_evaluator_fn,
            ptr: (handler.get_evaluator_fn)(handler.data, &schema, &expression),
        };
        Arc::new(wrapper)
    }
}

impl<T> Drop for EngineTraitWrapper<T> {
    fn drop(&mut self) {
        (self.drop_fn)(self.data, self.ptr)
    }
}

impl EngineTableClient {
    fn get_expression_handler(&self) -> Arc<dyn ExpressionHandler> {
        let wrapper = EngineTraitWrapper {
            data: self.data,
            drop_fn: self.drop_expression_handler_fn,
            ptr: (self.get_expression_handler_fn)(self.data),
        };
        Arc::new(wrapper)
    }
}

/*
impl TableClient for EngineTableClient {
    fn get_expression_handler(&self) -> Arc<dyn ExpressionHandler> {
        self.expression_handler(self.data)

    /// Get the connector provided [`FileSystemClient`]
    fn get_file_system_client(&self) -> Arc<dyn FileSystemClient>;

    /// Get the connector provided [`JsonHandler`].
    fn get_json_handler(&self) -> Arc<dyn JsonHandler>;

    /// Get the connector provided [`ParquetHandler`].
    fn get_parquet_handler(&self) -> Arc<dyn ParquetHandler>;
}
*/

// WARNING: the visitor MUST NOT retain internal references to the c_char names passed to visitor methods
// TODO: other types, nullability
#[repr(C)]
pub struct EngineSchemaVisitor {
    // opaque state pointer
    data: *mut c_void,
    // Creates a new field list, optionally reserving capacity up front
    make_field_list: extern "C" fn(data: *mut c_void, reserve: usize) -> usize,
    // visitor methods that should instantiate and append the appropriate type to the field list
    visit_struct: extern "C" fn(
        data: *mut c_void,
        sibling_list_id: usize,
        name: *const c_char,
        child_list_id: usize,
    ) -> (),
    visit_string:
        extern "C" fn(data: *mut c_void, sibling_list_id: usize, name: *const c_char) -> (),
    visit_integer:
        extern "C" fn(data: *mut c_void, sibling_list_id: usize, name: *const c_char) -> (),
    visit_long: extern "C" fn(data: *mut c_void, sibling_list_id: usize, name: *const c_char) -> (),
}

/// Model iterators. This allows an engine to specify iteration however it likes, and we simply wrap
/// the engine functions. The engine retains ownership of the iterator.
#[repr(C)]
pub struct EngineIterator {
    // Opaque data that will be iterated over. This data will be passed to the get_next function
    // each time a next item is requested from the iterator
    data: *mut c_void,
    // A function that should advance the iterator and return the next time from the data
    get_next: extern "C" fn(data: *mut c_void) -> *const c_void,
}

/// test function to print for items. this assumes each item is an `int`
#[no_mangle]
extern "C" fn iterate(it: &mut EngineIterator) {
    for i in it {
        let i = i as *mut i32;
        let ii = unsafe { &*i };
        println!("Got an item: {:?}", ii);
    }
}

impl Iterator for EngineIterator {
    // Todo: Figure out item type
    type Item = *const c_void;

    fn next(&mut self) -> Option<Self::Item> {
        let next_item = (self.get_next)(self.data);
        if next_item.is_null() {
            None
        } else {
            Some(next_item)
        }
    }
}

/// Whatever we decide this should be
pub struct ColumnBatch;

/// A struct with function pointers for all the operations a FileSystemClient must support
#[repr(C)]
pub struct FileSystemClientOps {
    list_from: extern "C" fn(path: *const c_char) -> *mut EngineIterator,
}

/// A struct with function pointers for all the operations a JsonHandler must support
#[repr(C)]
pub struct JsonHandlerOps {
    read_json_files:
        extern "C" fn(files: *const *const c_char, file_count: c_int) -> *const ColumnBatch, // schema?
}

/// A struct with function pointers for all the operations a top level client must perform
#[repr(C)]
pub struct EngineClientOps {
    get_file_system_client: extern "C" fn() -> *const FileSystemClientOps,
}

// stuff for the default client
use crate::client::executor::tokio::TokioBackgroundExecutor;
use crate::schema::{DataType, PrimitiveType, StructField, StructType};
use crate::snapshot::Snapshot;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

type DefaultSnapshot = Snapshot;
type KernelDefaultTableClient = crate::DefaultTableClient<TokioBackgroundExecutor>;

#[no_mangle]
pub extern "C" fn test_client(table_client: &Box<&dyn TableClient>) -> () {
    println!("Table client!")
}

/// # Safety
///
/// Caller is responsible to pass a valid path pointer.
unsafe fn unwrap_and_parse_path_as_url(path: *const c_char) -> Option<Url> {
    let path = unsafe { CStr::from_ptr(path) };
    let path = path.to_str().unwrap();
    let path = std::fs::canonicalize(PathBuf::from(path));
    let Ok(path) = path else {
        println!("Couldn't open table: {}", path.err().unwrap());
        return None
    };
    let Ok(url) = Url::from_directory_path(path) else {
        println!("Invalid url");
        return None
    };
    Some(url)
}

#[no_mangle]
pub unsafe extern "C" fn get_default_client(path: *const c_char) -> *const KernelDefaultTableClient {
    let path = unsafe { unwrap_and_parse_path_as_url(path) };
    let Some(url) = path else {
        return std::ptr::null_mut();
    };
    let table_client = KernelDefaultTableClient::try_new(
        &url,
        HashMap::<String, String>::new(),
        Arc::new(TokioBackgroundExecutor::new()),
    );
    let Ok(table_client) = table_client else {
        println!("Error creating table client: {}", table_client.err().unwrap());
        return std::ptr::null_mut();
    };
    Arc::into_raw(Arc::new(table_client))
}

/// Get the latest snapshot from the specified table
#[no_mangle]
pub unsafe extern "C" fn snapshot(path: *const c_char, table_client: &KernelDefaultTableClient) -> *const DefaultSnapshot {
    let path = unsafe { unwrap_and_parse_path_as_url(path) };
    let Some(url) = path else {
        return std::ptr::null_mut();
    };
    let snapshot = Snapshot::try_new(url, table_client, None).unwrap();
    Arc::into_raw(snapshot)
}

/// Get the version of the specified snapshot
#[no_mangle]
pub extern "C" fn version(snapshot: &DefaultSnapshot) -> u64 {
    snapshot.version()
}

#[no_mangle]
pub extern "C" fn visit_schema(
    snapshot: &DefaultSnapshot,
    table_client: &KernelDefaultTableClient,
    visitor: &mut EngineSchemaVisitor,
) -> usize {
    // Visit all the fields of a struct and return the list of children
    fn visit_struct_fields(visitor: &EngineSchemaVisitor, s: &StructType) -> usize {
        let child_list_id = (visitor.make_field_list)(visitor.data, s.fields.len());
        for field in s.fields.iter() {
            visit_field(visitor, child_list_id, field);
        }
        child_list_id
    }

    // Visit a struct field (recursively) and add the result to the list of siblings.
    fn visit_field(visitor: &EngineSchemaVisitor, sibling_list_id: usize, field: &StructField) {
        let name = CString::new(field.name.as_bytes()).unwrap();
        match &field.data_type {
            DataType::Primitive(PrimitiveType::Integer) => {
                (visitor.visit_integer)(visitor.data, sibling_list_id, name.as_ptr())
            }
            DataType::Primitive(PrimitiveType::Long) => {
                (visitor.visit_long)(visitor.data, sibling_list_id, name.as_ptr())
            }
            DataType::Primitive(PrimitiveType::String) => {
                (visitor.visit_string)(visitor.data, sibling_list_id, name.as_ptr())
            }
            DataType::Struct(s) => {
                let child_list_id = visit_struct_fields(visitor, s);
                (visitor.visit_struct)(visitor.data, sibling_list_id, name.as_ptr(), child_list_id);
            }
            other => println!("Unsupported data type: {}", other),
        }
    }

    // TODO: Snapshot should eagerly compute P&M so we don't need a table client here.
    let schema: StructType = snapshot.schema(table_client).unwrap();
    visit_struct_fields(visitor, &schema)
}

// A set that can identify its contents by address
pub struct ReferenceSet<T> {
    map: std::collections::HashMap<usize, T>,
    next_id: usize,
}

impl<T> ReferenceSet<T> {
    pub fn new() -> Self {
        Default::default()
    }

    // Inserts a new value into the set. This always creates a new entry
    // because the new value cannot have the same address as any existing value.
    // Returns a raw pointer to the value. This pointer serves as a key that
    // can be used later to take() from the set, and should NOT be dereferenced.
    pub fn insert(&mut self, value: T) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.map.insert(id, value);
        id
    }

    // Attempts to remove a value from the set, if present.
    pub fn take(&mut self, i: usize) -> Option<T> {
        self.map.remove(&i)
    }

    // True if the set contains an object whose address matches the pointer.
    pub fn contains(&self, id: usize) -> bool {
        self.map.contains_key(&id)
    }

    // The current size of the set.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl<T> Default for ReferenceSet<T> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            next_id: 1,
        }
    }
}

#[derive(Default)]
pub struct KernelExpressionVisitorState {
    // TODO: ReferenceSet<Box<dyn MetadataFilterFn>> instead?
    inflight_expressions: ReferenceSet<Expression>,
}
impl KernelExpressionVisitorState {
    fn new() -> Self {
        Self {
            inflight_expressions: Default::default(),
        }
    }
}

// When invoking [[get_scan_files]], The engine provides a pointer to the (engine's native)
// predicate, along with a visitor function that can be invoked to recursively visit the
// predicate. This engine state is valid until the call to [[get_scan_files]] returns. Inside that
// method, the kernel allocates visitor state, which becomes the second argument to the predicate
// visitor invocation along with the engine-provided predicate pointer. The visitor state is valid
// for the lifetime of the predicate visitor invocation. Thanks to this double indirection, engine
// and kernel each retain ownership of their respective objects, with no need to coordinate memory
// lifetimes with the other.
#[repr(C)]
pub struct EnginePredicate {
    predicate: *mut c_void,
    visitor:
        extern "C" fn(predicate: *mut c_void, state: &mut KernelExpressionVisitorState) -> usize,
}

fn wrap_expression(state: &mut KernelExpressionVisitorState, expr: Expression) -> usize {
    state.inflight_expressions.insert(expr)
}

fn unwrap_c_string(s: *const c_char) -> String {
    let s = unsafe { CStr::from_ptr(s) };
    s.to_str().unwrap().to_string()
}

fn unwrap_kernel_expression(
    state: &mut KernelExpressionVisitorState,
    exprid: usize,
) -> Option<Box<Expression>> {
    state.inflight_expressions.take(exprid).map(Box::new)
}

fn visit_expression_binary(
    state: &mut KernelExpressionVisitorState,
    op: BinaryOperator,
    a: usize,
    b: usize,
) -> usize {
    let left = unwrap_kernel_expression(state, a);
    let right = unwrap_kernel_expression(state, b);
    match left.zip(right) {
        Some((left, right)) => {
            wrap_expression(state, Expression::BinaryOperation { op, left, right })
        }
        None => 0, // invalid child => invalid node
    }
}

// The EngineIterator is not thread safe, not reentrant, not owned by callee, not freed by callee.
#[no_mangle]
pub extern "C" fn visit_expression_and(
    state: &mut KernelExpressionVisitorState,
    children: &mut EngineIterator,
) -> usize {
    let mut children = children.flat_map(|child| unwrap_kernel_expression(state, child as usize));
    let left = match children.next() {
        Some(left) => left,
        _ => return 0,
    };
    let right = match children.next() {
        Some(right) => right,
        _ => return wrap_expression(state, *left),
    };
    let mut result = Expression::BinaryOperation {
        op: BinaryOperator::And,
        left,
        right,
    };
    for child in children {
        let left = Box::new(result);
        result = Expression::BinaryOperation {
            op: BinaryOperator::And,
            left,
            right: child,
        };
    }
    wrap_expression(state, result)
}

#[no_mangle]
pub extern "C" fn visit_expression_lt(
    state: &mut KernelExpressionVisitorState,
    a: usize,
    b: usize,
) -> usize {
    visit_expression_binary(state, BinaryOperator::LessThan, a, b)
}

#[no_mangle]
pub extern "C" fn visit_expression_le(
    state: &mut KernelExpressionVisitorState,
    a: usize,
    b: usize,
) -> usize {
    visit_expression_binary(state, BinaryOperator::LessThanOrEqual, a, b)
}

#[no_mangle]
pub extern "C" fn visit_expression_gt(
    state: &mut KernelExpressionVisitorState,
    a: usize,
    b: usize,
) -> usize {
    visit_expression_binary(state, BinaryOperator::GreaterThan, a, b)
}

#[no_mangle]
pub extern "C" fn visit_expression_ge(
    state: &mut KernelExpressionVisitorState,
    a: usize,
    b: usize,
) -> usize {
    visit_expression_binary(state, BinaryOperator::GreaterThanOrEqual, a, b)
}

#[no_mangle]
pub extern "C" fn visit_expression_eq(
    state: &mut KernelExpressionVisitorState,
    a: usize,
    b: usize,
) -> usize {
    visit_expression_binary(state, BinaryOperator::Equal, a, b)
}

#[no_mangle]
pub extern "C" fn visit_expression_column(
    state: &mut KernelExpressionVisitorState,
    name: *const c_char,
) -> usize {
    wrap_expression(state, Expression::Column(unwrap_c_string(name)))
}

#[no_mangle]
pub extern "C" fn visit_expression_literal_string(
    state: &mut KernelExpressionVisitorState,
    value: *const c_char,
) -> usize {
    wrap_expression(
        state,
        Expression::Literal(Scalar::from(unwrap_c_string(value))),
    )
}

#[no_mangle]
pub extern "C" fn visit_expression_literal_long(
    state: &mut KernelExpressionVisitorState,
    value: i64,
) -> usize {
    wrap_expression(state, Expression::Literal(Scalar::from(value)))
}

#[repr(C)]
pub struct FileList {
    files: *mut *mut c_char,
    file_count: i32,
}

/// Get a FileList for all the files that need to be read from the table. NB: This _consumes_ the
/// snapshot, it is no longer valid after making this call (TODO: We should probably fix this?)
///
/// # Safety
///
/// Caller is responsible to pass a valid snapshot pointer.
#[no_mangle]
pub unsafe extern "C" fn get_scan_files(
    snapshot: *const DefaultSnapshot,
    table_client: *const KernelDefaultTableClient,
    predicate: Option<&mut EnginePredicate>,
) -> FileList {
    let snapshot: Arc<DefaultSnapshot> = unsafe { Arc::from_raw(snapshot) };
    let table_client = unsafe { Arc::from_raw(table_client) };
    let mut scan_builder = ScanBuilder::new(snapshot);
    if let Some(predicate) = predicate {
        // TODO: There is a lot of redundancy between the various visit_expression_XXX methods here,
        // vs. ProvidesMetadataFilter trait and the class hierarchy that supports it. Can we justify
        // combining the two, so that native rust kernel code also uses the visitor idiom? Doing so
        // might mean kernel no longer needs to define an expression class hierarchy of its own (at
        // least, not for data skipping). Things may also look different after we remove arrow code
        // from the kernel proper and make it one of the sensible default engine clients instead.
        let mut visitor_state = KernelExpressionVisitorState::new();
        let exprid = (predicate.visitor)(predicate.predicate, &mut visitor_state);
        if let Some(predicate) = unwrap_kernel_expression(&mut visitor_state, exprid) {
            println!("Got predicate: {}", predicate);
            scan_builder = scan_builder.with_predicate(*predicate);
        }
    }
    let scan_adds = scan_builder.build(table_client.as_ref()).unwrap().files(table_client.as_ref()).unwrap();
    let mut file_count = 0;
    let mut files: Vec<*mut i8> = scan_adds
        .into_iter()
        .map(|add| {
            file_count += 1;
            CString::new(add.unwrap().path).unwrap().into_raw()
        })
        .collect();
    let ptr = files.as_mut_ptr();
    std::mem::forget(files);
    println!("{} files survived pruning", file_count);
    FileList {
        files: ptr,
        file_count,
    }
}
