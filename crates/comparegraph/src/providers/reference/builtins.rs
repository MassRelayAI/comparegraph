//! Per-language names of builtins and common standard-library methods. A bare call to one of these never binds by name to a
//! same-named function in another file: it is the language's own function, not ours. Same-file and imported bindings are
//! decided before this list is consulted.

const PYTHON: &[&str] = &[
    "abs", "all", "any", "ascii", "bin", "bool", "bytearray", "bytes", "callable", "chr", "classmethod", "compile", "complex",
    "delattr", "dict", "dir", "divmod", "enumerate", "eval", "exec", "filter", "float", "format", "frozenset", "getattr",
    "globals", "hasattr", "hash", "help", "hex", "id", "input", "int", "isinstance", "issubclass", "iter", "len", "list",
    "locals", "map", "max", "memoryview", "min", "next", "object", "oct", "open", "ord", "pow", "print", "property", "range",
    "repr", "reversed", "round", "set", "setattr", "slice", "sorted", "staticmethod", "str", "sum", "super", "tuple", "type",
    "vars", "zip", "append", "extend", "insert", "remove", "pop", "clear", "copy", "update", "get", "keys", "values", "items",
    "join", "split", "strip", "lstrip", "rstrip", "replace", "startswith", "endswith", "lower", "upper", "encode", "decode",
    "read", "write", "close", "seek", "flush", "readline", "readlines", "add", "discard", "index", "count", "sort", "reverse",
    "find", "format_map", "setdefault", "__init__", "__new__", "__call__", "__enter__", "__exit__", "__str__", "__repr__",
    "__len__", "__iter__", "__next__", "__getitem__", "__setitem__", "__eq__", "__hash__", "__del__", "__getattr__",
];

const JS: &[&str] = &[
    "parseInt", "parseFloat", "isNaN", "isFinite", "setTimeout", "setInterval", "clearTimeout", "clearInterval", "require",
    "fetch", "String", "Number", "Boolean", "Array", "Object", "Symbol", "BigInt", "Date", "RegExp", "Error", "Promise", "Map",
    "Set", "WeakMap", "WeakSet", "JSON", "Math", "encodeURIComponent", "decodeURIComponent", "encodeURI", "decodeURI",
    "push", "pop", "shift", "unshift", "slice", "splice", "concat", "join", "map", "filter", "reduce", "reduceRight", "forEach",
    "find", "findIndex", "findLast", "some", "every", "includes", "indexOf", "lastIndexOf", "sort", "reverse", "flat", "flatMap",
    "fill", "keys", "values", "entries", "from", "of", "isArray", "assign", "freeze", "toString", "valueOf", "hasOwnProperty",
    "split", "trim", "trimStart", "trimEnd", "replace", "replaceAll", "match", "matchAll", "search", "substring", "substr",
    "startsWith", "endsWith", "charAt", "charCodeAt", "padStart", "padEnd", "repeat", "toLowerCase", "toUpperCase", "then",
    "catch", "finally", "resolve", "reject", "all", "allSettled", "race", "get", "set", "has", "delete", "clear", "add", "on",
    "emit", "call", "apply", "bind", "log", "warn", "error", "info", "debug", "stringify", "parse", "now", "floor", "ceil",
    "round", "max", "min", "abs", "random", "constructor",
];

const RUST: &[&str] = &[
    "new", "len", "is_empty", "get", "get_mut", "insert", "remove", "push", "pop", "iter", "iter_mut", "into_iter", "clone",
    "unwrap", "expect", "unwrap_or", "unwrap_or_else", "unwrap_or_default", "map", "map_err", "and_then", "or_else", "ok",
    "err", "ok_or", "ok_or_else", "is_some", "is_none", "is_ok", "is_err", "as_ref", "as_mut", "as_str", "as_bytes",
    "to_string", "to_owned", "to_vec", "into", "from", "try_from", "try_into", "default", "drop", "take", "replace", "extend",
    "collect", "filter", "fold", "sum", "count", "next", "first", "last", "contains", "contains_key", "entry", "or_insert",
    "keys", "values", "join", "split", "trim", "lines", "chars", "parse", "format", "write", "read", "flush", "lock", "borrow",
    "borrow_mut", "min", "max", "cmp", "eq", "ne", "fmt", "hash", "deref", "with_capacity", "capacity", "clear", "sort",
    "sort_by", "dedup", "retain", "truncate", "starts_with", "ends_with", "find", "any", "all", "zip", "enumerate", "rev",
    "skip", "chain", "flatten", "Some", "None", "Ok", "Err", "Box", "Vec", "String", "Option", "Result",
];

const GO: &[&str] = &[
    "append", "cap", "close", "complex", "copy", "delete", "imag", "len", "make", "new", "panic", "print", "println", "real",
    "recover", "min", "max", "clear",
];

const JAVA: &[&str] = &[
    "toString", "equals", "hashCode", "getClass", "clone", "finalize", "notify", "notifyAll", "wait", "compareTo", "length",
    "size", "isEmpty", "get", "set", "add", "put", "remove", "contains", "containsKey", "iterator", "next", "hasNext", "stream",
    "map", "filter", "collect", "forEach", "of", "valueOf", "format", "println", "print", "append", "substring", "indexOf",
    "charAt", "trim", "split", "replace", "startsWith", "endsWith", "toLowerCase", "toUpperCase", "parseInt", "close", "run",
    "start", "join", "sleep", "getMessage", "printStackTrace", "orElse", "isPresent", "ifPresent", "build", "builder",
];

/// Is `name` a builtin or common standard-library name of this (family) language? Unknown languages have no list.
pub fn is_builtin(language: &str, name: &str) -> bool {
    let list = match language {
        "python" => PYTHON,
        "typescript" | "tsx" | "javascript" | "jsx" | "js" => JS,
        "rust" => RUST,
        "go" => GO,
        "java" => JAVA,
        _ => return false,
    };
    list.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_are_per_language() {
        assert!(is_builtin("python", "len"));
        assert!(is_builtin("rust", "is_empty"));
        assert!(!is_builtin("python", "is_empty"));
        assert!(!is_builtin("python", "search_symbols"));
        assert!(!is_builtin("cobol", "len"));
    }
}
