//! CMP-UI-LOOP round 3: `ui owners` / `ui screens-of` join a registry screen to the files of the root repo's web app.
//! The fixture is a small web app: a router file with route literals (nested, a layout route, a redirect, a commented-out route, a catch-all),
//! pages that import components through the `~/` alias (the stored graph does not resolve it) and relative paths. Each test was seen red first.
mod common;
use common::{cc, clone_with, run};
use serde_json::{json, Value};
use std::path::Path;

const APP: &str = r#"import { Routes, Route, Navigate } from "react-router-dom";
import { Shell } from "~/components/Shell";
import { Home } from "~/pages/Home";
import { Room } from "./pages/Room";
import { Advanced } from "~/pages/Advanced";
import { NotFound } from "~/pages/NotFound";

export function App() {
  return (
    <Routes>
      <Route element={<Shell />}>
        <Route path="/" element={<Home />} />
        <Route path="/rooms/:id" element={<Room />} />
        <Route path="/settings">
          <Route path="advanced" element={<Advanced />} />
        </Route>
        <Route path="/old" element={<Navigate to="/rooms/1" replace />} />
        {/* <Route path="/ghost" element={<Ghost />} /> */}
      </Route>
      <Route path="*" element={<NotFound />} />
    </Routes>
  );
}
"#;
const OTHER_APP: &str = r#"import { Routes, Route } from "react-router-dom";
import { OtherHome } from "../../pages/OtherHome";
export function OtherApp() {
  return (<Routes><Route path="/" element={<OtherHome />} /></Routes>);
}
"#;
const SHELL: &str = "import { Rail } from \"~/components/Rail\";\nexport function Shell() { return <div><Rail /><main /></div>; }\n";
const RAIL: &str = "import { Strip } from \"./Strip\";\nexport function Rail() { return <nav><Strip /></nav>; }\n";
const STRIP: &str = "export function Strip() { return <i />; }\n";
const HOME: &str = "import { RoomsList } from \"~/pages/RoomsList\";\nexport function Home() { return <RoomsList />; }\n";
const ROOMS_LIST: &str = "export function RoomsList() { return <ul />; }\n";
const ROOM: &str = "import type { Foo } from \"~/lib/types\";\nimport { Composer } from \"~/components/Composer\";\nimport { helper } from \"~/lib/helper\";\nimport { sample } from \"~/components/Composer.test\";\nimport { Passed } from \"~/components/Passed\";\nexport function Room() { helper(); return <Composer extra={Passed} />; }\n";
const COMPOSER: &str = "import { L2 } from \"./L2\";\nexport function Composer() { return <L2 />; }\n";
const L2: &str = "import { L3 } from \"./L3\";\nexport function L2() { return <L3 />; }\n";
const L3: &str = "import { L4 } from \"./L4\";\nexport function L3() { return <L4 />; }\n";
const L4: &str = "export function L4() { return <b />; }\n";
const PASSED: &str = "export function Passed() { return <u />; }\n";
const ADVANCED: &str = "export function Advanced() { return <section />; }\n";
const NOT_FOUND: &str = "export function NotFound() { return <p />; }\n";
const OTHER_HOME: &str = "export function OtherHome() { return <p />; }\n";
const TEST: &str = "import { Composer } from \"./Composer\";\nit(\"x\", () => { Composer(); });\n";

fn registry() -> Value {
    json!({
        "schema": "ui-registry/1",
        "screens": [
            { "id": "room" },
            { "id": "invite-person", "ours": { "currentRoutes": ["/rooms/:id (Invite)"] } },
            { "id": "home", "ours": { "currentRoutes": ["/", "/start"] } },
            { "id": "rooms-list" },
            { "id": "settings-advanced" },
            { "id": "old" },
            { "id": "ghost" },
            { "id": "nowhere" }
        ],
        "products": { "massrelay": { "routes": [
            { "base": "app", "pattern": "/rooms/:id", "screen": "room" },
            { "base": "app", "pattern": "/rooms", "screen": "rooms-list" },
            { "base": "app", "pattern": "/settings/advanced", "screen": "settings-advanced" },
            { "base": "app", "pattern": "/old", "screen": "old" },
            { "base": "app", "pattern": "/ghost", "screen": "ghost" }
        ] } }
    })
}

struct Fx {
    ws: tempfile::TempDir,
    ui: tempfile::TempDir,
    _clone: tempfile::TempDir,
}
fn fx() -> Fx {
    let clone = tempfile::tempdir().unwrap();
    let w = "apps/web/src";
    let files: Vec<(String, &str)> = vec![
        ("App.tsx", APP),
        ("surfaces/other/OtherApp.tsx", OTHER_APP),
        ("components/Shell.tsx", SHELL),
        ("components/Rail.tsx", RAIL),
        ("components/Strip.tsx", STRIP),
        ("pages/Home.tsx", HOME),
        ("pages/RoomsList.tsx", ROOMS_LIST),
        ("pages/Room.tsx", ROOM),
        ("components/Composer.tsx", COMPOSER),
        ("components/L2.tsx", L2),
        ("components/L3.tsx", L3),
        ("components/L4.tsx", L4),
        ("components/Passed.tsx", PASSED),
        ("pages/Advanced.tsx", ADVANCED),
        ("pages/NotFound.tsx", NOT_FOUND),
        ("components/Composer.test.tsx", TEST),
        ("lib/helper.ts", "export function helper() {}\n"),
    ]
    .into_iter()
    .map(|(p, c)| (format!("{w}/{p}"), c))
    .collect();
    let mut all: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    all.push(("apps/web/src/pages/OtherHome.tsx", OTHER_HOME));
    clone_with(clone.path(), &all, "MIT");
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["add"]).arg(clone.path()).args(["--name", "massrelay", "--as-root"]).assert().success();
    cc(ws.path()).args(["build", "massrelay"]).assert().success();
    let ui = tempfile::tempdir().unwrap();
    std::fs::write(ui.path().join("registry.json"), serde_json::to_string_pretty(&registry()).unwrap()).unwrap();
    Fx { ws, ui, _clone: clone }
}
fn ui(f: &Fx, args: &[&str]) -> Value {
    let mut a = vec!["ui", "--ui-root", f.ui.path().to_str().unwrap()];
    a.extend_from_slice(args);
    run(f.ws.path(), &a)
}
fn codes(v: &Value) -> Vec<String> {
    v["evidence"]["limitations"].as_array().map(|a| a.iter().filter_map(|x| x["code"].as_str().map(String::from)).collect()).unwrap_or_default()
}
fn files_of(v: &Value) -> Vec<String> {
    v["data"]["owners"].as_array().unwrap().iter().map(|o| o["file"].as_str().unwrap().trim_start_matches("apps/web/src/").to_string()).collect()
}
fn owner<'a>(v: &'a Value, file: &str) -> &'a Value {
    v["data"]["owners"].as_array().unwrap().iter().find(|o| o["file"].as_str().unwrap().ends_with(file)).unwrap_or_else(|| panic!("{file} not an owner: {v}"))
}

#[test]
fn owners_page_first_then_children_by_depth_with_edge_kind_and_confidence() {
    let f = fx();
    let v = ui(&f, &["owners", "room"]);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["pageFiles"], json!(["apps/web/src/pages/Room.tsx"]));
    let files = files_of(&v);
    assert_eq!(files[0], "pages/Room.tsx");
    let room = owner(&v, "pages/Room.tsx");
    assert_eq!((room["how"].as_str(), room["confidence"].as_str(), room["scope"].as_str(), room["rank"].as_u64()), (Some("route-element"), Some("exact"), Some("page"), Some(1)));
    assert_eq!(room["via"], "/rooms/:id in apps/web/src/App.tsx:13");
    let comp = owner(&v, "components/Composer.tsx");
    assert_eq!((comp["how"].as_str(), comp["depth"].as_u64()), (Some("renders"), Some(1)));
    // imported but only passed as a prop: an import edge, ranked after the renders at the same depth
    let passed = owner(&v, "components/Passed.tsx");
    assert_eq!((passed["how"].as_str(), passed["depth"].as_u64()), (Some("imports"), Some(1)));
    assert!(files.iter().position(|x| x == "components/Composer.tsx") < files.iter().position(|x| x == "components/Passed.tsx"));
    // depth-bounded at 3: L2 d2, L3 d3, L4 (d4) absent; no test file, no .ts helper, no type import
    assert_eq!(owner(&v, "components/L3.tsx")["depth"], 3);
    for gone in ["L4.tsx", "Composer.test.tsx", "helper.ts", "types"] {
        assert!(!files.iter().any(|x| x.ends_with(gone)), "{gone} must not be an owner: {files:?}");
    }
    assert_eq!(v["evidence"]["confidence"], "derived");
    // the layout comes after every page-scope file, with its own closure
    let shell = files.iter().position(|x| x == "components/Shell.tsx").unwrap();
    assert!(files.iter().position(|x| x == "components/L3.tsx").unwrap() < shell);
    assert_eq!(owner(&v, "components/Shell.tsx")["how"], "layout-element");
    assert_eq!((owner(&v, "components/Rail.tsx")["scope"].as_str(), owner(&v, "components/Rail.tsx")["depth"].as_u64()), (Some("layout"), Some(1)));
    assert_eq!(owner(&v, "components/Strip.tsx")["depth"], 2);
    assert!(codes(&v).contains(&"root_built_at".to_string()), "{:?}", codes(&v));
    // the `~/` alias is not an import edge in the stored graph: said, not hidden
    assert!(codes(&v).contains(&"graph_imports_partial".to_string()), "{:?}", codes(&v));
}

#[test]
fn owners_depth_flag_bounds_the_closure() {
    let f = fx();
    let v = ui(&f, &["owners", "room", "--depth", "1"]);
    let files = files_of(&v);
    assert!(files.contains(&"components/Composer.tsx".to_string()));
    assert!(!files.iter().any(|x| x.ends_with("L2.tsx")), "{files:?}");
    assert_eq!(ui(&f, &["owners", "room", "--depth", "9"])["ok"], false);
}

#[test]
fn a_screen_without_a_route_literal_is_route_unmapped_never_guessed() {
    let f = fx();
    for screen in ["rooms-list", "ghost", "nowhere"] {
        let v = ui(&f, &["owners", screen]);
        assert_eq!(v["ok"], true, "{screen}: {v}");
        assert_eq!(v["data"]["owners"], json!([]), "{screen}");
        assert_eq!(v["evidence"]["confidence"], "unknown", "{screen}");
        assert!(codes(&v).contains(&"route_unmapped".to_string()), "{screen}: {:?}", codes(&v));
    }
    // the commented-out route and the catch-all map nothing
    let v = ui(&f, &["owners", "ghost"]);
    assert_eq!(v["data"]["routes"][0]["status"], "route_unmapped");
    assert!(v["data"]["routes"][0]["reason"].as_str().unwrap().contains("catch-all /*"), "{v}");
    assert_eq!(ui(&f, &["owners", "no-such-screen"])["error"]["code"], "not_found");
}

#[test]
fn nested_relative_routes_are_joined_and_registry_prose_is_stripped() {
    let f = fx();
    let v = ui(&f, &["owners", "settings-advanced"]);
    assert_eq!(v["data"]["pageFiles"], json!(["apps/web/src/pages/Advanced.tsx"]), "{v}");
    assert_eq!(v["data"]["routes"][0]["matches"][0]["tableRoute"], "/settings/advanced");
    // `/rooms/:id (Invite)` is the route /rooms/:id
    let v = ui(&f, &["owners", "invite-person"]);
    assert_eq!(v["data"]["routes"][0]["registryRoute"], "/rooms/:id");
    assert_eq!(v["data"]["pageFiles"], json!(["apps/web/src/pages/Room.tsx"]));
}

#[test]
fn several_registry_routes_and_a_redirect_with_pattern_confidence() {
    let f = fx();
    let v = ui(&f, &["owners", "home"]);
    // `/` is mapped, `/start` has no literal
    assert_eq!(v["data"]["routes"][0]["status"], "mapped");
    assert_eq!(v["data"]["routes"][1]["status"], "route_unmapped");
    assert!(files_of(&v).contains(&"pages/RoomsList.tsx".to_string()), "Home renders RoomsList: {v}");
    assert!(codes(&v).contains(&"route_unmapped".to_string()));
    // `/old` redirects (one hop) to /rooms/1, which only a pattern route matches
    let v = ui(&f, &["owners", "old"]);
    let m = &v["data"]["routes"][0]["matches"][0];
    assert_eq!((m["redirectFrom"].as_str(), m["confidence"].as_str(), m["tableRoute"].as_str()), (Some("/old"), Some("pattern"), Some("/rooms/:id")), "{v}");
    assert_eq!(v["evidence"]["confidence"], "inferred");
}

#[test]
fn a_parameter_or_splat_match_is_pattern_and_the_envelope_is_inferred() {
    let f = fx();
    std::fs::write(
        f.ui.path().join("registry.json"),
        serde_json::to_string(&json!({ "screens": [{ "id": "r1" }], "products": { "massrelay": { "routes": [{ "pattern": "/rooms/42", "screen": "r1" }] } } })).unwrap(),
    )
    .unwrap();
    let v = ui(&f, &["owners", "r1"]);
    assert_eq!(v["data"]["routes"][0]["matches"][0]["confidence"], "pattern", "{v}");
    assert_eq!(owner(&v, "pages/Room.tsx")["confidence"], "pattern");
    assert_eq!(v["evidence"]["confidence"], "inferred");
    assert!(codes(&v).contains(&"route_pattern_match".to_string()));
}

#[test]
fn router_selection_defaults_to_the_app_root_and_router_flag_picks_another() {
    let f = fx();
    std::fs::write(
        f.ui.path().join("registry.json"),
        serde_json::to_string(&json!({ "screens": [{ "id": "h" }], "products": { "massrelay": { "routes": [{ "pattern": "/", "screen": "h" }] } } })).unwrap(),
    )
    .unwrap();
    let v = ui(&f, &["owners", "h"]);
    assert_eq!(v["data"]["pageFiles"], json!(["apps/web/src/pages/Home.tsx"]), "{v}");
    assert!(codes(&v).contains(&"router_selected".to_string()));
    let v = ui(&f, &["owners", "h", "--router", "apps/web/src/surfaces/other/OtherApp.tsx"]);
    assert_eq!(v["data"]["pageFiles"], json!(["apps/web/src/pages/OtherHome.tsx"]), "{v}");
}

#[test]
fn screens_of_is_the_reverse_through_page_child_and_layout() {
    let f = fx();
    let screens = |file: &str| -> Value { ui(&f, &["screens-of", file]) };
    let by = |v: &Value, id: &str| -> Value { v["data"]["screens"].as_array().unwrap().iter().find(|s| s["screen"] == id).cloned().unwrap_or(Value::Null) };
    // a page: itself the page of the room screens
    let v = screens("apps/web/src/pages/Room.tsx");
    assert_eq!(v["ok"], true, "{v}");
    let r = by(&v, "room");
    assert_eq!((r["role"].as_str(), r["depth"].as_u64(), r["how"].as_str()), (Some("page"), Some(0), Some("route-element")), "{v}");
    assert_eq!(by(&v, "invite-person")["role"], "page");
    assert!(by(&v, "home").is_null());
    // a child: Composer at depth 1 of Room, and L4 is beyond the default depth
    let v = screens("apps/web/src/components/Composer.tsx");
    let r = by(&v, "room");
    assert_eq!((r["role"].as_str(), r["depth"].as_u64(), r["how"].as_str()), (Some("child"), Some(1), Some("renders")));
    assert!(screens("apps/web/src/components/L4.tsx")["data"]["screens"].as_array().unwrap().is_empty());
    // a layout child: Rail is rendered by Shell, which wraps every mapped screen
    let v = screens("apps/web/src/components/Rail.tsx");
    let r = by(&v, "home");
    assert_eq!((r["role"].as_str(), r["depth"].as_u64()), (Some("layout-child"), Some(1)), "{v}");
    assert!(!by(&v, "room").is_null() && !by(&v, "settings-advanced").is_null());
    // the screens whose route was not found are named: their blast radius is unknown
    let un: Vec<&str> = v["data"]["unmapped"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(un.iter().any(|x| x.starts_with("rooms-list")) && un.iter().any(|x| x.starts_with("ghost")), "{un:?}");
    assert!(codes(&v).contains(&"route_unmapped".to_string()));
    // Windows separators and an unknown file
    assert_eq!(screens("apps\\web\\src\\pages\\Room.tsx")["ok"], true);
    assert_eq!(screens("apps/web/src/pages/Nope.tsx")["error"]["code"], "not_found");
}

#[test]
fn the_answer_is_deterministic_and_the_tools_are_in_the_catalog() {
    let f = fx();
    assert_eq!(ui(&f, &["owners", "room"]), ui(&f, &["owners", "room"]));
    assert_eq!(ui(&f, &["screens-of", "apps/web/src/components/Rail.tsx"]), ui(&f, &["screens-of", "apps/web/src/components/Rail.tsx"]));
    let out = cc(Path::new(".")).arg("catalog").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    for t in ["comparegraph_ui_owners", "comparegraph_ui_screens_of"] {
        assert!(s.contains(t), "{t} missing from the catalog");
    }
}

#[test]
fn no_registry_or_no_root_repo_is_a_clear_error() {
    let f = fx();
    std::fs::remove_file(f.ui.path().join("registry.json")).unwrap();
    assert_eq!(ui(&f, &["owners", "room"])["error"]["code"], "not_found");
    let empty = tempfile::tempdir().unwrap();
    let v = run(empty.path(), &["ui", "--ui-root", f.ui.path().to_str().unwrap(), "owners", "room"]);
    assert_eq!(v["ok"], false);
}

/// A success answer carries every key its advertised outputSchema requires, with the declared JSON type (the conformance test in
/// output_schema.rs only sees the error answer of these two tools: its fixture root repo has no web app).
#[test]
fn success_answers_match_the_advertised_output_schema() {
    let f = fx();
    let cat: Value = serde_json::from_str(include_str!("../../../docs/catalog.json")).unwrap();
    for (tool, args) in [("comparegraph_ui_owners", vec!["owners", "room"]), ("comparegraph_ui_screens_of", vec!["screens-of", "apps/web/src/pages/Room.tsx"])] {
        let t = cat["tools"].as_array().unwrap().iter().find(|t| t["name"] == tool).unwrap();
        let ok_branch = t["outputSchema"]["anyOf"].as_array().unwrap().iter().find(|b| b["properties"]["ok"]["const"] == true).unwrap();
        let data_schema = &ok_branch["properties"]["data"];
        let v = ui(&f, &args);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["verb"], t["outputSchema"]["properties"]["verb"]["const"]);
        for k in data_schema["required"].as_array().unwrap() {
            let k = k.as_str().unwrap();
            let ty = data_schema["properties"][k]["type"].as_str().unwrap_or("");
            let got = &v["data"][k];
            assert!(!got.is_null(), "{tool}: required key {k} missing in {}", v["data"]);
            match ty {
                "string" => assert!(got.is_string(), "{tool}.{k}"),
                "integer" => assert!(got.is_u64() || got.is_i64(), "{tool}.{k}"),
                "array" => assert!(got.is_array(), "{tool}.{k}"),
                "object" => assert!(got.is_object(), "{tool}.{k}"),
                _ => {}
            }
        }
    }
}
