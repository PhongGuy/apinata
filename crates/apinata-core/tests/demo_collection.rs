//! The committed demo collection is exactly the writer's output, loads cleanly and survives
//! an edit-and-revert save. Set APINATA_REGEN_DEMO=1 to rewrite examples/demo from the builder.

use apinata_core::{
    Assertion, AuthSpec, BodySpec, Collection, ContextOptions, EnvVariable, Folder, ItemPath,
    IterationData, KeyValue, MemoryStore, OAuth1Spec, Param, ParamKind, PostResponseVar,
    RequestFile, RequestKind, TagFilter, Timeout, VarContext, VarValue, Variable, resolve,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo")
}

fn files(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).expect("under root").to_path_buf();
                out.insert(rel, fs::read_to_string(&path).expect("utf-8 file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn copy_tree(from: &Path, to: &Path) {
    for (rel, text) in files(from) {
        let dest = to.join(rel);
        fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
        fs::write(dest, text).expect("write");
    }
}

fn all_requests(folder: &Folder, out: &mut Vec<PathBuf>) {
    out.extend(folder.requests().iter().map(|r| r.path().to_path_buf()));
    for sub in folder.folders() {
        all_requests(sub, out);
    }
}

const BASE_URL: &str = "https://jsonplaceholder.typicode.com";

/// Create a request in `folder`, apply `edit` to it and save it.
fn add_request(
    c: &mut Collection,
    folder: &ItemPath,
    name: &str,
    method: &str,
    url: &str,
    edit: impl FnOnce(&mut RequestFile),
) {
    let item = c.create_request(folder, name).expect("create request");
    let r = c.request_mut(&item).expect("new request");
    r.set_method(method);
    r.set_url(url);
    keep_no_limit(r);
    edit(r);
    r.save().expect("save request");
}

/// The committed demo's requests carry an explicit no-limit timeout (`0`), which new requests
/// no longer get, so the builder sets it to keep examples/demo unchanged.
fn keep_no_limit(r: &mut RequestFile) {
    let mut settings = r.settings();
    settings.timeout = Timeout::Unlimited;
    r.set_settings(settings);
}

/// Create a GraphQL request in `folder` posting `query` and `variables` to `url`, and save it.
fn add_graphql_request(
    c: &mut Collection,
    folder: &ItemPath,
    name: &str,
    url: &str,
    query: &str,
    variables: &str,
) {
    let item = c
        .create_request_of_kind(folder, name, RequestKind::Graphql)
        .expect("create GraphQL request");
    let r = c.request_mut(&item).expect("new request");
    r.set_url(url);
    keep_no_limit(r);
    r.set_body(BodySpec::Graphql {
        query: query.to_owned(),
        variables: variables.to_owned(),
    });
    r.save().expect("save request");
}

fn status_is(code: &str) -> Assertion {
    Assertion::new("res.status", "eq", Some(code.to_owned()))
}

fn smoke() -> Vec<String> {
    vec!["smoke".to_owned()]
}

fn json(body: &str) -> BodySpec {
    BodySpec::Json(body.to_owned())
}

/// Build the JSONPlaceholder demo at `root` through the public writer API only, so the
/// committed files are whatever a user's own edits would produce.
fn build_demo(root: &Path) {
    let mut c = Collection::create(root, "Apinata Demo").expect("create collection");
    c.file_mut().set_docs(format!(
        "# Apinata Demo\n\nA small tour of Apinata.\n\n\
         - **Posts** and **Users** call [JSONPlaceholder]({BASE_URL}), a free fake REST API.\n\
         - **Media** calls [httpbin](https://httpbin.org) for responses that are not JSON.\n\
         - **Auth** shows digest, NTLM, AWS Sig v4 and OAuth 1.0 signatures that httpbin echoes.\n\
         - **GraphQL** calls [GraphQLZero](https://graphqlzero.almansi.me), JSONPlaceholder's data over GraphQL.\n\n\
         Sending a request needs network access."
    ));
    // Collection variables make every request work with no environment selected; the
    // environment then shows how one overrides a collection value.
    c.file_mut().set_vars(vec![
        Variable::new("baseUrl", VarValue::Text(BASE_URL.to_owned())),
        Variable::new("postId", VarValue::Text("1".to_owned())),
        Variable::new("userId", VarValue::Text("1".to_owned())),
    ]);
    c.file_mut().save().expect("save collection");

    let (env, _) = c
        .create_environment("JSONPlaceholder", &MemoryStore::new())
        .expect("create environment");
    let env = c.environment_mut(&env).expect("new environment");
    env.set_variables(vec![EnvVariable::new(
        "baseUrl",
        VarValue::Text(BASE_URL.to_owned()),
    )]);
    env.save().expect("save environment");

    let posts = c.create_folder(&ItemPath::root(), "Posts").expect("posts");
    add_request(
        &mut c,
        &posts,
        "List posts",
        "GET",
        "{{baseUrl}}/posts",
        |r| {
            r.set_params(vec![Param::new("_limit", "5", ParamKind::Query)]);
            r.set_assertions(vec![status_is("200")]);
            r.set_tags(smoke());
        },
    );
    add_request(
        &mut c,
        &posts,
        "Get post",
        "GET",
        "{{baseUrl}}/posts/:id",
        |r| {
            r.set_params(vec![Param::new("id", "{{postId}}", ParamKind::Path)]);
            r.set_assertions(vec![
                status_is("200"),
                Assertion::new("res.body.id", "eq", Some("1".to_owned())),
            ]);
            r.set_post_response_vars(vec![PostResponseVar::new("postTitle", "res.body.title")]);
            r.set_tags(smoke());
            r.set_docs("Fetches the post named by the `postId` collection variable and keeps its title as `postTitle`.");
        },
    );
    add_request(
        &mut c,
        &posts,
        "Create post",
        "POST",
        "{{baseUrl}}/posts",
        |r| {
            r.set_headers(vec![KeyValue::new("Content-Type", "application/json")])
                .expect("valid headers");
            r.set_body(json(
            "{\n  \"title\": \"Hello from Apinata\",\n  \"body\": \"A demo post.\",\n  \"userId\": 1\n}",
        ));
            r.set_assertions(vec![status_is("201")]);
        },
    );
    add_request(
        &mut c,
        &posts,
        "Replace post",
        "PUT",
        "{{baseUrl}}/posts/1",
        |r| {
            r.set_body(json(
            "{\n  \"id\": 1,\n  \"title\": \"Replaced\",\n  \"body\": \"A replaced post.\",\n  \"userId\": 1\n}",
        ));
        },
    );
    add_request(
        &mut c,
        &posts,
        "Edit post title",
        "PATCH",
        "{{baseUrl}}/posts/1",
        |r| {
            r.set_body(json("{\n  \"title\": \"Edited\"\n}"));
        },
    );
    add_request(
        &mut c,
        &posts,
        "Delete post",
        "DELETE",
        "{{baseUrl}}/posts/1",
        |r| {
            r.set_assertions(vec![status_is("200")]);
        },
    );

    let users = c.create_folder(&ItemPath::root(), "Users").expect("users");
    add_request(
        &mut c,
        &users,
        "List users",
        "GET",
        "{{baseUrl}}/users",
        |r| {
            r.set_headers(vec![KeyValue::new("Accept", "application/json")])
                .expect("valid headers");
            r.set_assertions(vec![status_is("200")]);
            r.set_tags(smoke());
        },
    );
    // A data file run repeats this request once per row, each row overriding `userId`.
    add_request(
        &mut c,
        &users,
        "Get user",
        "GET",
        "{{baseUrl}}/users/{{userId}}",
        |r| {
            r.set_assertions(vec![status_is("200")]);
            r.set_tags(vec!["smoke".to_owned(), "data".to_owned()]);
            r.set_docs(
                "Fetches the user named by `userId`. Run it with `examples/demo-data/users.csv` \
                 as the data file to fetch users 1 to 3, one per iteration.",
            );
        },
    );

    let media = c.create_folder(&ItemPath::root(), "Media").expect("media");
    let folder = c.folder_file_mut(&media).expect("media folder file");
    folder.set_docs(
        "## Media\n\nResponses that are not JSON:\n\n\
         1. **Get XML** returns an XML document, shown re-indented and highlighted.\n\
         2. **Get PNG** returns a PNG image, shown as a preview with its size and dimensions.",
    );
    folder.save().expect("save media folder");
    for (name, url) in [
        ("Get XML", "https://httpbin.org/xml"),
        ("Get PNG", "https://httpbin.org/image/png"),
    ] {
        add_request(&mut c, &media, name, "GET", url, |r| {
            r.set_assertions(vec![status_is("200")]);
        });
    }

    // httpbin answers digest challenges and echoes any signed request; the AWS keys are the
    // documentation's example pair, which httpbin accepts since it checks no signature.
    let auth = c.create_folder(&ItemPath::root(), "Auth").expect("auth");
    add_request(
        &mut c,
        &auth,
        "Digest example",
        "GET",
        "https://httpbin.org/digest-auth/auth/user/passwd",
        |r| {
            r.set_auth(AuthSpec::Digest {
                username: "user".to_owned(),
                password: "passwd".to_owned(),
            });
            r.set_assertions(vec![status_is("200")]);
        },
    );
    add_request(
        &mut c,
        &auth,
        "SigV4 example",
        "GET",
        "https://httpbin.org/anything",
        |r| {
            r.set_auth(AuthSpec::AwsV4 {
                access_key_id: "AKIDEXAMPLE".to_owned(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned(),
                session_token: String::new(),
                service: "execute-api".to_owned(),
                region: "us-east-1".to_owned(),
                profile_name: String::new(),
            });
            r.set_assertions(vec![status_is("200")]);
        },
    );
    add_request(
        &mut c,
        &auth,
        "OAuth 1.0 example",
        "GET",
        "https://httpbin.org/get",
        |r| {
            let mut o = OAuth1Spec::default();
            o.consumer_key = Some("demo-consumer-key".to_owned());
            o.consumer_secret = Some("demo-consumer-secret".to_owned());
            o.access_token = Some("demo-token".to_owned());
            o.access_token_secret = Some("demo-token-secret".to_owned());
            for field in [
                &mut o.callback_url,
                &mut o.verifier,
                &mut o.timestamp,
                &mut o.nonce,
                &mut o.realm,
            ] {
                *field = None;
            }
            o.private_key = None;
            r.set_auth(AuthSpec::OAuth1(o));
            r.set_assertions(vec![status_is("200")]);
        },
    );
    // httpbin offers no NTLM challenge, so the request is answered as is; a real server
    // that asks for NTLM completes the handshake with these example credentials.
    add_request(
        &mut c,
        &auth,
        "NTLM example",
        "GET",
        "https://httpbin.org/get",
        |r| {
            r.set_auth(AuthSpec::Ntlm {
                username: "demo".to_owned(),
                password: "demo-password".to_owned(),
                domain: "EXAMPLE".to_owned(),
            });
            r.set_assertions(vec![status_is("200")]);
        },
    );

    // GraphQLZero serves the JSONPlaceholder data over GraphQL.
    let graphql = c
        .create_folder(&ItemPath::root(), "GraphQL")
        .expect("graphql");
    add_graphql_request(
        &mut c,
        &graphql,
        "Get user",
        "https://graphqlzero.almansi.me/api",
        "query ($id: ID!) {\n  user(id: $id) {\n    id\n    name\n    email\n  }\n}",
        "{\n  \"id\": 1\n}",
    );
}

#[test]
fn the_committed_demo_is_the_writers_output() {
    let tmp = TempDir::new().expect("tempdir");
    let built = tmp.path().join("demo");
    build_demo(&built);
    if std::env::var_os("APINATA_REGEN_DEMO").is_some() {
        let _ = fs::remove_dir_all(demo_dir());
        copy_tree(&built, &demo_dir());
    }
    let (want, got) = (files(&built), files(&demo_dir()));
    assert_eq!(
        want.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>(),
        "file set differs; rerun with APINATA_REGEN_DEMO=1"
    );
    for (rel, text) in &want {
        assert_eq!(&got[rel], text, "{} differs from the writer", rel.display());
    }
}

#[test]
fn the_demo_loads_without_errors() {
    let c = Collection::open(demo_dir()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let mut requests = Vec::new();
    all_requests(c.tree(), &mut requests);
    assert_eq!(requests.len(), 15);
    let envs: Vec<&str> = c.environments().iter().map(|e| e.name()).collect();
    assert_eq!(envs, ["JSONPlaceholder"]);
    let text: String = files(&demo_dir()).into_values().collect();
    assert!(!text.contains("secret: true"));
    // The format's own extension key is not user text; any other mention is.
    let text = text.replace("extensions:\n  bruno:\n", "extensions:\n");
    assert!(!text.to_lowercase().contains("bruno"));
}

#[test]
fn the_demo_holds_one_graphql_request() {
    let c = Collection::open(demo_dir()).expect("opens");
    let r = c
        .request(&ItemPath::new("GraphQL/Get user.yml").unwrap())
        .expect("the GraphQL demo request");
    assert_eq!(r.kind(), RequestKind::Graphql);
    assert_eq!(r.url(), "https://graphqlzero.almansi.me/api");
}

#[test]
fn editing_and_reverting_each_request_restores_its_bytes() {
    let tmp = TempDir::new().expect("tempdir");
    copy_tree(&demo_dir(), tmp.path());
    let before = files(tmp.path());
    let mut c = Collection::open(tmp.path()).expect("opens");
    let mut paths = Vec::new();
    all_requests(c.tree(), &mut paths);
    for path in paths {
        let item: ItemPath = c.item_path(&path).expect("item");
        let r = c.request_mut(&item).expect("request");
        let url = r.url().to_owned();
        r.set_url(format!("{url}?probe=1"));
        r.save().expect("save edit");
        r.set_url(url);
        r.save().expect("save revert");
    }
    assert_eq!(files(tmp.path()), before);
}

#[test]
fn demo_urls_resolve_with_no_environment_selected() {
    let c = Collection::open(demo_dir()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let get_post = c.root().join("Posts/Get post.yml");
    let item = c.item_path(&get_post).expect("item");
    let req = c.request(&item).expect("request");
    let url = resolve(req, &ctx).expect("resolves").url().to_owned();
    assert_eq!(url, format!("{BASE_URL}/posts/1"));
}

#[test]
fn the_demo_shows_xml_an_image_and_markdown_docs() {
    let c = Collection::open(demo_dir()).expect("opens");
    let mut requests = Vec::new();
    all_requests(c.tree(), &mut requests);
    let names: Vec<String> = requests
        .iter()
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    for name in ["Get XML", "Get PNG"] {
        assert!(names.iter().any(|n| n == name), "{names:?}");
    }
    let folder = fs::read_to_string(demo_dir().join("Media/folder.yml")).expect("folder");
    assert!(folder.contains("## Media"), "{folder}");
    let root = fs::read_to_string(demo_dir().join("opencollection.yml")).expect("root");
    assert!(root.contains("# Apinata Demo"), "{root}");
    assert!(root.contains("httpbin.org"), "{root}");
}

#[test]
fn demo_auth_requests_resolve_with_no_undefined_variables() {
    let c = Collection::open(demo_dir()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    for name in [
        "Auth/Digest example.yml",
        "Auth/SigV4 example.yml",
        "Auth/OAuth 1.0 example.yml",
        "Auth/NTLM example.yml",
    ] {
        let item = c.item_path(&c.root().join(name)).expect("item");
        let req = c.request(&item).expect("request");
        resolve(req, &ctx).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn the_demo_ntlm_request_carries_its_credentials_in_the_file() {
    let c = Collection::open(demo_dir()).expect("opens");
    let item = c
        .item_path(&c.root().join("Auth/NTLM example.yml"))
        .expect("item");
    let req = c.request(&item).expect("request");
    assert_eq!(
        req.auth(),
        AuthSpec::Ntlm {
            username: "demo".to_owned(),
            password: "demo-password".to_owned(),
            domain: "EXAMPLE".to_owned(),
        }
    );
    let text = fs::read_to_string(demo_dir().join("Auth/NTLM example.yml")).expect("file");
    assert!(
        text.contains("type: ntlm") && text.contains("domain: EXAMPLE"),
        "{text}"
    );
}

#[test]
fn the_demo_tags_select_a_smoke_run() {
    let c = Collection::open(demo_dir()).expect("opens");
    let plan = c.plan(c.root()).expect("plan");
    let smoke = plan.filter_tags(&TagFilter::new(vec!["smoke".to_owned()], Vec::new()));
    let names: Vec<&str> = smoke.requests().map(|r| r.name()).collect();
    assert_eq!(names, ["List posts", "Get post", "List users", "Get user"]);
}

#[test]
fn the_demo_data_file_feeds_get_user() {
    let text = fs::read_to_string(demo_dir().join("../demo-data/users.csv")).expect("data file");
    let data = IterationData::from_csv(&text).expect("parses");
    assert_eq!(data.len(), 3);
    assert_eq!(
        data.row(2).expect("row"),
        [("userId".to_owned(), "3".to_owned())]
    );
}

// Every demo request goes to a public service that answers, never to a placeholder host, so
// a plain run of the demo can pass.
#[test]
fn every_demo_request_targets_a_service_that_answers() {
    const ANSWERING: [&str; 3] = [
        "jsonplaceholder.typicode.com",
        "httpbin.org",
        "graphqlzero.almansi.me",
    ];
    let c = Collection::open(demo_dir()).expect("opens");
    let plan = c.plan(c.root()).expect("plan");
    let mut seen = 0;
    for request in plan.requests() {
        let url = request.url().replace("{{baseUrl}}", BASE_URL);
        let host = url
            .strip_prefix("https://")
            .and_then(|rest| rest.split(['/', ':', '?']).next())
            .unwrap_or_default();
        assert!(
            ANSWERING.contains(&host),
            "{} calls {host:?}, which is not a service known to answer",
            request.name()
        );
        seen += 1;
    }
    assert_eq!(seen, 15);
}
