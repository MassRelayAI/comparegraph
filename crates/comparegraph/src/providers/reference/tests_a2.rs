//! Plan 5 A2: receivers typed by a DECLARED return annotation or a DECLARED field/attribute type. Never from a body.
use super::ReferenceProvider;
use crate::domain::EdgeKind;
use crate::providers::{BuildOptions, BuildOutput, GraphProvider};

fn build(files: &[(&str, &str)]) -> BuildOutput {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let full = d.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    ReferenceProvider.build(d.path(), &BuildOptions::default()).unwrap()
}
fn conf(o: &BuildOutput, a: &str, b: &str) -> Option<f64> {
    o.edges_by_qn.iter().find(|e| e.0 == EdgeKind::Calls && e.1 == a && e.2 == b).map(|e| e.3)
}
/// Calls from `a` into a method named `read` (the receiver-typed targets these tests are about).
fn calls_from(o: &BuildOutput, a: &str) -> usize {
    o.edges_by_qn.iter().filter(|e| e.0 == EdgeKind::Calls && e.1 == a && e.2.contains("read")).count()
}

const PY_READER: &str = "class Reader:\n    def read(self):\n        pass\nclass Other:\n    def read(self):\n        pass\n";

#[test]
fn python_call_result_typed_by_return_annotation() {
    let o = build(&[
        ("r.py", PY_READER),
        ("a.py", "from r import Reader\ndef make() -> Reader:\n    return Reader()\ndef go():\n    x = make()\n    x.read()\n"),
    ]);
    assert_eq!(conf(&o, "a.py::go", "r.py::read"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn python_imported_factory_typed_by_its_return_annotation() {
    let o = build(&[
        ("r.py", PY_READER),
        ("f.py", "from r import Reader\ndef make() -> Reader:\n    return Reader()\n"),
        ("a.py", "from f import make\ndef go():\n    x = make()\n    x.read()\n"),
    ]);
    assert_eq!(conf(&o, "a.py::go", "r.py::read"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn python_tuple_unpacking_from_a_tuple_annotation() {
    for ann in ["tuple[Reader, Other]", "Tuple[Reader, Other]"] {
        let src = format!("from r import Reader, Other\ndef pair() -> {ann}:\n    return Reader(), Other()\ndef go():\n    a, b = pair()\n    a.read()\n    b.read()\n");
        let o = build(&[("r.py", PY_READER), ("a.py", &src)]);
        assert_eq!(conf(&o, "a.py::go", "r.py::read"), Some(1.0), "{ann}: {:?}", o.edges_by_qn);
        assert_eq!(calls_from(&o, "a.py::go"), 2, "{ann}: two reads: {:?}", o.edges_by_qn);
        assert!(conf(&o, "a.py::go", "r.py::read#2").is_some(), "{ann}: second read on Other: {:?}", o.edges_by_qn);
    }
}

#[test]
fn python_unannotated_factory_types_nothing() {
    let o = build(&[
        ("r.py", PY_READER),
        ("a.py", "from r import Reader\ndef make():\n    return Reader()\ndef go():\n    x = make()\n    x.read()\n"),
    ]);
    assert!(conf(&o, "a.py::go", "r.py::read").is_none() && conf(&o, "a.py::go", "r.py::read#2").is_none(), "{:?}", o.edges_by_qn);
}

#[test]
fn python_external_annotation_types_nothing() {
    let o = build(&[
        ("r.py", PY_READER),
        ("a.py", "import sqlite3\ndef make() -> sqlite3.Connection:\n    pass\ndef go():\n    x = make()\n    x.read()\n"),
    ]);
    assert!(conf(&o, "a.py::go", "r.py::read").is_none(), "{:?}", o.edges_by_qn);
}

#[test]
fn python_self_attribute_from_annotation_and_constructor() {
    let o = build(&[
        ("r.py", PY_READER),
        (
            "a.py",
            "from r import Reader, Other\nclass App:\n    def __init__(self):\n        self.a: Reader = None\n        self.b = Other()\n    def go(self):\n        self.a.read()\n        self.b.read()\n",
        ),
    ]);
    assert_eq!(conf(&o, "a.py::go", "r.py::read"), Some(1.0), "{:?}", o.edges_by_qn);
    assert_eq!(conf(&o, "a.py::go", "r.py::read#2"), Some(1.0), "{:?}", o.edges_by_qn);
}

const TS_READER: &str = "export class Reader {\n  read() {}\n}\nexport class Other {\n  read() {}\n}\n";

#[test]
fn ts_call_result_typed_by_return_annotation() {
    let o = build(&[
        ("r.ts", TS_READER),
        ("a.ts", "import { Reader } from './r';\nfunction make(): Reader {\n  return new Reader();\n}\nfunction go() {\n  const x = make();\n  x.read();\n}\n"),
    ]);
    assert_eq!(conf(&o, "a.ts::go", "r.ts::read"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn ts_unannotated_function_and_external_type_type_nothing() {
    let o = build(&[
        ("r.ts", TS_READER),
        ("a.ts", "import { Reader } from './r';\nfunction make() {\n  return new Reader();\n}\nfunction go() {\n  const x = make();\n  x.read();\n}\n"),
        ("b.ts", "import { Conn } from 'db';\nfunction make(): Conn {\n  return null as any;\n}\nfunction go2() {\n  const x = make();\n  x.read();\n}\n"),
    ]);
    assert!(conf(&o, "a.ts::go", "r.ts::read").is_none(), "{:?}", o.edges_by_qn);
    assert!(conf(&o, "b.ts::go2", "r.ts::read").is_none(), "{:?}", o.edges_by_qn);
}

#[test]
fn ts_class_fields_type_this_dot_field_receivers() {
    let o = build(&[
        ("r.ts", TS_READER),
        (
            "a.ts",
            "import { Reader, Other } from './r';\nexport class App {\n  private reader: Reader;\n  other = new Other();\n  go() {\n    this.reader.read();\n    this.other.read();\n  }\n}\n",
        ),
    ]);
    assert_eq!(conf(&o, "a.ts::go", "r.ts::read"), Some(1.0), "{:?}", o.edges_by_qn);
    assert_eq!(conf(&o, "a.ts::go", "r.ts::read#2"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn ts_constructor_parameter_property_types_the_field() {
    let o = build(&[
        ("r.ts", TS_READER),
        ("a.ts", "import { Reader } from './r';\nexport class App {\n  constructor(private reader: Reader) {}\n  go() {\n    this.reader.read();\n  }\n}\n"),
    ]);
    assert_eq!(conf(&o, "a.ts::go", "r.ts::read"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn ts_untyped_and_external_fields_type_nothing() {
    let o = build(&[
        ("r.ts", TS_READER),
        ("a.ts", "import { Conn } from 'db';\nexport class App {\n  private c: Conn;\n  private u;\n  go() {\n    this.c.read();\n    this.u.read();\n  }\n}\n"),
    ]);
    assert_eq!(calls_from(&o, "a.ts::go"), 0, "{:?}", o.edges_by_qn);
}

const RS_LIB: &str = "pub mod store;\npub mod app;\n";
const RS_STORE: &str = "pub struct Reader;\nimpl Reader {\n    pub fn read(&self) {}\n}\npub struct Other;\nimpl Other {\n    pub fn read(&self) {}\n}\n";

#[test]
fn rust_call_result_typed_by_declared_return_in_the_same_crate() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("src/lib.rs", RS_LIB),
        ("src/store.rs", RS_STORE),
        ("src/app.rs", "use crate::store::Reader;\nfn make() -> Reader {\n    Reader\n}\npub fn go() {\n    let x = make();\n    x.read();\n}\n"),
    ]);
    assert_eq!(conf(&o, "src/app.rs::go", "src/store.rs::read"), Some(1.0), "{:?}", o.edges_by_qn);
}

#[test]
fn rust_unannotated_or_external_return_types_nothing() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("src/lib.rs", RS_LIB),
        ("src/store.rs", RS_STORE),
        ("src/app.rs", "use crate::store::Reader;\nfn make() {\n    let _r = Reader;\n}\nfn ext() -> Vec<u8> {\n    vec![]\n}\nfn ext2() -> Connection {\n    todo!()\n}\npub fn go() {\n    let x = make();\n    x.read();\n    let y = ext2();\n    y.read();\n}\n"),
    ]);
    assert_eq!(calls_from(&o, "src/app.rs::go"), 0, "{:?}", o.edges_by_qn);
}

#[test]
fn rust_struct_fields_type_self_dot_field_receivers() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("src/lib.rs", RS_LIB),
        ("src/store.rs", RS_STORE),
        (
            "src/app.rs",
            "use crate::store::Reader;\npub struct App {\n    pub reader: Reader,\n    conn: Connection,\n}\nimpl App {\n    pub fn go(&self) {\n        self.reader.read();\n        self.conn.read();\n    }\n}\n",
        ),
    ]);
    assert_eq!(conf(&o, "src/app.rs::go", "src/store.rs::read"), Some(1.0), "{:?}", o.edges_by_qn);
    assert_eq!(calls_from(&o, "src/app.rs::go"), 1, "{:?}", o.edges_by_qn);
}

#[test]
fn python_tuple_of_the_wrong_arity_or_without_annotation_types_nothing() {
    let o = build(&[
        ("r.py", PY_READER),
        (
            "a.py",
            "from r import Reader, Other\ndef pair() -> tuple[Reader, Other]:\n    pass\ndef bare():\n    pass\ndef go():\n    a, b, c = pair()\n    a.read()\n    d, e = bare()\n    d.read()\n}\n",
        ),
    ]);
    assert_eq!(calls_from(&o, "a.py::go"), 0, "{:?}", o.edges_by_qn);
}

#[test]
fn ts_field_type_does_not_leak_to_a_plain_local_of_the_same_name() {
    let o = build(&[
        ("r.ts", TS_READER),
        ("a.ts", "import { Reader } from './r';\nexport class App {\n  private reader: Reader;\n  go(reader: any) {\n    reader.read();\n  }\n}\n"),
    ]);
    assert_eq!(calls_from(&o, "a.ts::go"), 0, "{:?}", o.edges_by_qn);
}
