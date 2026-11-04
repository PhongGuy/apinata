//! The web globals (`URL`, `URLSearchParams`) that scripts get beyond the language itself.

use rquickjs::{Ctx, Exception, Function};
use url::{Url, quirks};

use super::engine::Installer;

const WEB_JS: &str = include_str!("web.js");

/// Installs the `__url_parse` and `__url_set` ops, then evaluates `web.js`, which builds the
/// globals over them and deletes the ops.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        ctx.globals()
            .set("__url_parse", Function::new(ctx.clone(), parse)?)?;
        ctx.globals()
            .set("__url_set", Function::new(ctx.clone(), set)?)?;
        ctx.eval::<(), _>(WEB_JS)
    })
}

/// The WHATWG components of `url` as a JSON object of strings.
fn components(url: &Url) -> String {
    serde_json::json!({
        "href": quirks::href(url),
        "origin": quirks::origin(url),
        "protocol": quirks::protocol(url),
        "username": quirks::username(url),
        "password": quirks::password(url),
        "host": quirks::host(url),
        "hostname": quirks::hostname(url),
        "port": quirks::port(url),
        "pathname": quirks::pathname(url),
        "search": quirks::search(url),
        "hash": quirks::hash(url),
    })
    .to_string()
}

/// Parses `input` against an optional `base`; any failure, including an unparsable base,
/// is the `TypeError: Invalid URL` the standard names.
fn parse<'js>(ctx: Ctx<'js>, input: String, base: Option<String>) -> rquickjs::Result<String> {
    let invalid = || Exception::throw_type(&ctx, "Invalid URL");
    let base = match base {
        Some(base) => Some(Url::parse(&base).map_err(|_| invalid())?),
        None => None,
    };
    let url = Url::options()
        .base_url(base.as_ref())
        .parse(&input)
        .map_err(|_| invalid())?;
    Ok(components(&url))
}

/// Applies one setter to the URL at `href`. A setter the standard ignores leaves the URL as it
/// was; `href` re-parses and throws when the new value is invalid.
fn set<'js>(ctx: Ctx<'js>, href: String, field: String, value: String) -> rquickjs::Result<String> {
    let invalid = || Exception::throw_type(&ctx, "Invalid URL");
    if field == "href" {
        return parse(ctx.clone(), value, None);
    }
    let mut url = Url::parse(&href).map_err(|_| invalid())?;
    // The fallible setters report a rejected value by `Err`; the URL is untouched then.
    let _ = match field.as_str() {
        "protocol" => quirks::set_protocol(&mut url, &value),
        "username" => quirks::set_username(&mut url, &value),
        "password" => quirks::set_password(&mut url, &value),
        "host" => quirks::set_host(&mut url, &value),
        "hostname" => quirks::set_hostname(&mut url, &value),
        "port" => quirks::set_port(&mut url, &value),
        "pathname" => {
            quirks::set_pathname(&mut url, &value);
            Ok(())
        }
        "search" => {
            quirks::set_search(&mut url, &value);
            Ok(())
        }
        "hash" => {
            quirks::set_hash(&mut url, &value);
            Ok(())
        }
        _ => return Err(Exception::throw_type(&ctx, "Unknown URL field")),
    };
    Ok(components(&url))
}
