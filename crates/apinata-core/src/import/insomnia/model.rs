//! The parts of an Insomnia export the importer reads. v4 resources and v5 items are both read
//! into `RequestSource` and `FolderSource`, which keep each member beside its input path. Each
//! struct's `rest` keeps the members it does not name, so they can be noted.

use crate::js;
use serde::Deserialize;
use serde_json::{Map, Value};

/// Members that carry no request data, so they are never noted as unread.
const EXEMPT: [&str; 14] = [
    "_id",
    "parentId",
    "created",
    "modified",
    "_type",
    "type",
    "metaSortKey",
    "isPrivate",
    "dataPropertyOrder",
    "environmentPropertyOrder",
    "color",
    "schema_version",
    "kvPairData",
    "environmentType",
];

/// A member nothing reads: its member path and the note detail.
pub(super) type Unread = (String, &'static str);

pub(super) const UNREAD: &str = "this member is not imported";
const NOT_OBJECT: &str = "not an object; not imported";

/// Whether `key` is export metadata that is never noted.
pub(super) fn exempt(key: &str) -> bool {
    EXEMPT.contains(&key) || key.starts_with("__export_")
}

/// The paths `<at><key>` of the keys in `rest` that are neither exempt nor in `skip`.
pub(super) fn rest_paths(
    rest: &Map<String, Value>,
    at: &str,
    skip: &[&str],
    out: &mut Vec<Unread>,
) {
    let unread = rest
        .keys()
        .filter(|k| !exempt(k) && !skip.contains(&k.as_str()));
    out.extend(unread.map(|k| (format!("{at}{k}"), UNREAD)));
}

/// The unread keys of an object member at `member.<key>`; a member that is not an object is
/// unread as a whole, and a missing or `null` one is nothing.
pub(super) fn leftovers(value: Option<&Value>, member: &str, skip: &[&str], out: &mut Vec<Unread>) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => rest_paths(map, &format!("{member}."), skip, out),
        Some(_) => out.push((member.to_owned(), NOT_OBJECT)),
    }
}

/// A v5 document, after its `type` has been checked.
#[derive(Deserialize)]
pub(super) struct Document {
    pub(super) name: Option<Value>,
    pub(super) meta: Option<Value>,
    pub(super) collection: Option<Value>,
    #[serde(rename = "cookieJar")]
    pub(super) cookie_jar: Option<Value>,
    pub(super) spec: Option<Value>,
    #[serde(rename = "testSuites")]
    pub(super) test_suites: Option<Value>,
    pub(super) environments: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// One entry of a v4 `resources` list.
#[derive(Deserialize, Default)]
pub(super) struct Resource {
    #[serde(rename = "_id")]
    pub(super) id: Option<Value>,
    #[serde(rename = "_type")]
    pub(super) kind: Option<Value>,
    #[serde(rename = "parentId")]
    pub(super) parent_id: Option<Value>,
    pub(super) name: Option<Value>,
    #[serde(rename = "metaSortKey")]
    pub(super) sort_key: Option<Value>,
    pub(super) description: Option<Value>,
    pub(super) data: Option<Value>,
    pub(super) environment: Option<Value>,
    pub(super) method: Option<Value>,
    pub(super) url: Option<Value>,
    pub(super) cookies: Option<Value>,
    pub(super) parameters: Option<Value>,
    #[serde(rename = "pathParameters")]
    pub(super) path_parameters: Option<Value>,
    pub(super) headers: Option<Value>,
    pub(super) authentication: Option<Value>,
    pub(super) body: Option<Value>,
    #[serde(rename = "preRequestScript")]
    pub(super) pre_request_script: Option<Value>,
    #[serde(rename = "afterResponseScript")]
    pub(super) after_response_script: Option<Value>,
    #[serde(rename = "settingEncodeUrl")]
    pub(super) setting_encode_url: Option<Value>,
    #[serde(rename = "settingFollowRedirects")]
    pub(super) setting_follow_redirects: Option<Value>,
    #[serde(rename = "settingSendCookies")]
    pub(super) setting_send_cookies: Option<Value>,
    #[serde(rename = "settingStoreCookies")]
    pub(super) setting_store_cookies: Option<Value>,
    #[serde(rename = "settingRebuildPath")]
    pub(super) setting_rebuild_path: Option<Value>,
    #[serde(rename = "settingDisableRenderRequestBody")]
    pub(super) setting_disable_render_request_body: Option<Value>,
    #[serde(rename = "disableUserAgentHeader")]
    pub(super) disable_user_agent_header: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// One entry of a v5 `collection` or `children` list.
#[derive(Deserialize, Default)]
pub(super) struct Item {
    pub(super) name: Option<Value>,
    pub(super) meta: Option<Value>,
    pub(super) method: Option<Value>,
    pub(super) url: Option<Value>,
    pub(super) children: Option<Value>,
    #[serde(rename = "reflectionApi")]
    pub(super) reflection_api: Option<Value>,
    pub(super) parameters: Option<Value>,
    #[serde(rename = "pathParameters")]
    pub(super) path_parameters: Option<Value>,
    pub(super) headers: Option<Value>,
    pub(super) authentication: Option<Value>,
    pub(super) body: Option<Value>,
    pub(super) scripts: Option<Value>,
    pub(super) settings: Option<Value>,
    pub(super) environment: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A member of the input and its member path, such as `url` or `meta.description`.
pub(super) struct Member {
    pub(super) value: Value,
    pub(super) path: String,
}

impl Member {
    pub(super) fn new(value: Option<Value>, path: &str) -> Option<Member> {
        value.map(|value| Member {
            value,
            path: path.to_owned(),
        })
    }
}

/// A request, from either version, under the v4 field names.
pub(super) struct RequestSource {
    pub(super) name: String,
    pub(super) method: Option<Member>,
    pub(super) url: Option<Member>,
    pub(super) parameters: Option<Member>,
    pub(super) path_parameters: Option<Member>,
    pub(super) headers: Option<Member>,
    pub(super) auth: Option<Member>,
    pub(super) body: Option<Member>,
    pub(super) description: Option<Member>,
    pub(super) scripts: Vec<Member>,
    pub(super) encode_url: Option<Member>,
    pub(super) follow_redirects: Option<Member>,
    /// Settings with no counterpart, each beside Insomnia's default for it.
    pub(super) other_settings: Vec<(Member, bool)>,
    pub(super) unread: Vec<Unread>,
}

/// A folder, from either version, under the v4 field names.
pub(super) struct FolderSource {
    pub(super) name: String,
    pub(super) description: Option<Member>,
    pub(super) headers: Option<Member>,
    pub(super) auth: Option<Member>,
    pub(super) scripts: Vec<Member>,
    pub(super) environment: Option<Member>,
    pub(super) unread: Vec<Unread>,
}

impl RequestSource {
    pub(super) fn from_v4(r: Resource) -> RequestSource {
        let other_settings = [
            (r.setting_send_cookies, "settingSendCookies", true),
            (r.setting_store_cookies, "settingStoreCookies", true),
            (r.setting_rebuild_path, "settingRebuildPath", true),
            (
                r.setting_disable_render_request_body,
                "settingDisableRenderRequestBody",
                false,
            ),
            (r.disable_user_agent_header, "disableUserAgentHeader", false),
        ];
        let mut unread = Vec::new();
        rest_paths(&r.rest, "", &[], &mut unread);
        RequestSource {
            name: name_or(r.name.as_ref(), "New Request"),
            method: Member::new(r.method, "method"),
            url: Member::new(r.url, "url"),
            parameters: Member::new(r.parameters, "parameters"),
            path_parameters: Member::new(r.path_parameters, "pathParameters"),
            headers: Member::new(r.headers, "headers"),
            auth: Member::new(r.authentication, "authentication"),
            body: Member::new(r.body, "body"),
            description: Member::new(r.description, "description"),
            scripts: v4_scripts(r.pre_request_script, r.after_response_script),
            encode_url: Member::new(r.setting_encode_url, "settingEncodeUrl"),
            follow_redirects: Member::new(r.setting_follow_redirects, "settingFollowRedirects"),
            other_settings: other_settings
                .into_iter()
                .filter_map(|(v, path, default)| Member::new(v, path).map(|m| (m, default)))
                .collect(),
            unread,
        }
    }

    pub(super) fn from_v5(mut item: Item) -> RequestSource {
        let settings = &mut item.settings;
        let encode_url = Member::new(take(settings, "encodeUrl"), "settings.encodeUrl");
        let follow_redirects = Member::new(
            take(settings, "followRedirects"),
            "settings.followRedirects",
        );
        let mut cookies = take(settings, "cookies");
        let other_settings = [
            (
                take(settings, "renderRequestBody"),
                "settings.renderRequestBody",
            ),
            (take(&mut cookies, "send"), "settings.cookies.send"),
            (take(&mut cookies, "store"), "settings.cookies.store"),
            (take(settings, "rebuildPath"), "settings.rebuildPath"),
        ];
        let mut unread = Vec::new();
        rest_paths(&item.rest, "", &[], &mut unread);
        leftovers(
            item.meta.as_ref(),
            "meta",
            &["id", "sortKey", "description"],
            &mut unread,
        );
        leftovers(item.settings.as_ref(), "settings", &[], &mut unread);
        leftovers(cookies.as_ref(), "settings.cookies", &[], &mut unread);
        leftovers(
            item.scripts.as_ref(),
            "scripts",
            &["preRequest", "afterResponse"],
            &mut unread,
        );
        // A request holds neither items nor variables, so these members of one are lost.
        let lost = [
            ("children", &item.children),
            ("environment", &item.environment),
        ];
        for (key, value) in lost {
            if value.as_ref().is_some_and(|v| !v.is_null()) {
                unread.push((key.to_owned(), UNREAD));
            }
        }
        RequestSource {
            name: name_or(item.name.as_ref(), "New Request"),
            method: Member::new(item.method, "method"),
            url: Member::new(item.url, "url"),
            parameters: Member::new(item.parameters, "parameters"),
            path_parameters: Member::new(item.path_parameters, "pathParameters"),
            headers: Member::new(item.headers, "headers"),
            auth: Member::new(item.authentication, "authentication"),
            body: Member::new(item.body, "body"),
            description: Member::new(take(&mut item.meta, "description"), "meta.description"),
            scripts: v5_scripts(item.scripts),
            encode_url,
            follow_redirects,
            // Every v5 setting without a counterpart defaults to on.
            other_settings: other_settings
                .into_iter()
                .filter_map(|(v, path)| Member::new(v, path).map(|m| (m, true)))
                .collect(),
            unread,
        }
    }
}

impl FolderSource {
    pub(super) fn from_v4(r: Resource) -> FolderSource {
        let mut unread = Vec::new();
        rest_paths(&r.rest, "", &[], &mut unread);
        FolderSource {
            name: name_or(r.name.as_ref(), "New Folder"),
            description: Member::new(r.description, "description"),
            headers: Member::new(r.headers, "headers"),
            auth: Member::new(r.authentication, "authentication"),
            scripts: v4_scripts(r.pre_request_script, r.after_response_script),
            environment: Member::new(r.environment, "environment"),
            unread,
        }
    }

    /// The folder and its `children` list; children that are not a list are noted instead.
    pub(super) fn from_v5(mut item: Item, name: String) -> (FolderSource, Vec<Value>) {
        let mut unread = Vec::new();
        let children = match item.children.take() {
            Some(Value::Array(list)) => list,
            None | Some(Value::Null) => Vec::new(),
            Some(_) => {
                unread.push(("children".to_owned(), "not a list; not imported"));
                Vec::new()
            }
        };
        rest_paths(&item.rest, "", &[], &mut unread);
        leftovers(
            item.meta.as_ref(),
            "meta",
            &["id", "sortKey", "description"],
            &mut unread,
        );
        leftovers(
            item.scripts.as_ref(),
            "scripts",
            &["preRequest", "afterResponse"],
            &mut unread,
        );
        let source = FolderSource {
            name,
            description: Member::new(take(&mut item.meta, "description"), "meta.description"),
            headers: Member::new(item.headers, "headers"),
            auth: Member::new(item.authentication, "authentication"),
            scripts: v5_scripts(item.scripts),
            environment: Member::new(item.environment, "environment"),
            unread,
        };
        (source, children)
    }
}

fn v4_scripts(pre: Option<Value>, after: Option<Value>) -> Vec<Member> {
    let pre = Member::new(pre, "preRequestScript");
    pre.into_iter()
        .chain(Member::new(after, "afterResponseScript"))
        .collect()
}

fn v5_scripts(mut scripts: Option<Value>) -> Vec<Member> {
    let pre = Member::new(take(&mut scripts, "preRequest"), "scripts.preRequest");
    let after = Member::new(take(&mut scripts, "afterResponse"), "scripts.afterResponse");
    pre.into_iter().chain(after).collect()
}

/// Move `key` out of an object member; anything else yields nothing.
fn take(object: &mut Option<Value>, key: &str) -> Option<Value> {
    object
        .as_mut()
        .and_then(Value::as_object_mut)
        .and_then(|o| o.remove(key))
}

/// A member as text: missing and `null` are empty, other scalars render as Insomnia shows them.
pub(super) fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_text(v),
    }
}

/// A name as written, or `default` when it is missing or blank.
pub(super) fn name_or(value: Option<&Value>, default: &str) -> String {
    let name = text(value);
    if name.trim().is_empty() {
        default.to_owned()
    } else {
        name
    }
}

/// A sort key; anything but a number sorts as 0.
pub(super) fn sort_key(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}
