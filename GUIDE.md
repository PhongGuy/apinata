# Apinata guide

The full reference for Apinata's collections, command line, mock server, desktop
app, importers, MCP server and releases. For what Apinata is, how to install it
and a quick start, see the [README](README.md).

## Contents

- [Working with collections](#working-with-collections)
  - [Naming the collection](#naming-the-collection)
  - [Git](#git)
  - [Search](#search)
  - [Check](#check)
  - [List](#list)
  - [Environments and secrets](#environments-and-secrets)
  - [Items and folders](#items-and-folders)
  - [Host overrides](#host-overrides)
  - [Replace](#replace)
- [Command line](#command-line)
  - [Exit codes](#exit-codes)
  - [Shell completions](#shell-completions)
  - [Running requests](#running-requests)
  - [Variables and environments](#variables-and-environments)
  - [Reports](#reports)
  - [Sending one request](#sending-one-request)
  - [Code generation](#code-generation)
  - [GraphQL schema](#graphql-schema)
  - [OAuth 2.0 and scripts](#oauth-20-and-scripts)
  - [Timeouts, streams and body size](#timeouts-streams-and-body-size)
  - [Reading responses](#reading-responses)
- [Mock server](#mock-server)
- [Desktop app](#desktop-app)
- [Importing and exporting](#importing-and-exporting)
- [Legacy .bru collections](#legacy-bru-collections)
- [MCP server](#mcp-server)
- [Updates](#updates)
- [Release files](#release-files)
- [Building and packaging](#building-and-packaging)

## Working with collections

A collection is a directory of OpenCollection YAML: one file per request, a
`folder.yml` per folder, `environments/*.yml`, and `opencollection.yml` at the
root. It is meant to live in git next to your code.

### Naming the collection

Every command that works on a collection takes it two ways, and they mean the
same: as a leading argument, or as `--collection <DIR>`. Without either, the
command uses the collection around the working directory. Naming two different
collections is a usage error (exit 3); naming the same one twice is fine. When
no collection is found, the error says which forms that command takes.

| Command | Positional form | Flag form |
| --- | --- | --- |
| `run`, `mock` | `<PATH>` (required) | `--collection <DIR>` |
| `check`, `mcp`, `git status`, `git branches` | `[COLLECTION]` | `--collection <DIR>` |
| `export` | `<COLLECTION>` (required) | `--collection <DIR>` |
| `ls` | `[COLLECTION] [PATH]` | `--collection <DIR>` |
| `search` | `[COLLECTION] <QUERY>` | `--collection <DIR>` |
| `replace` | `[COLLECTION] <PATTERN> <REPLACEMENT>` | `--collection <DIR>` |
| `env`, `trust`, `git switch` | none | `--collection <DIR>` |

`env`, `trust` and `git switch` take their own arguments first, so a leading
collection would be ambiguous and only the flag exists. In `ls`, one argument
is the folder to list, unless it is a directory holding
`opencollection.yml` that the working directory's collection has no folder of;
then it is the collection. Add a second argument, or use `--collection`, to
say which you mean.

### Git

`apinata git status` lists the collection's files changed since the last
commit, with the branch. `apinata git diff` shows one request file's changed
sections and a line diff against the last commit, with the path taken from
the working directory. Both only read git, and `--json` prints the same data
as JSON:

```sh
apinata git status ./my-api
cd my-api
apinata git diff users/get-user.yml
```

Without git, or outside a repository, both print a short note and exit 0 (see
the [exit codes](#exit-codes)). `apinata git clone` clones a repository into a
new folder, `apinata git branches` lists the remote branches and
`apinata git switch` checks one out. The desktop app marks changed requests in
the sidebar and tabs, and lists the changes with their diffs in a Changes
dialog.

### Search

`apinata search [collection] <text>` finds text in the collection's requests, as lines of
path, field and snippet, or with `--json` as data. It lists at most 500 hits
and says when more matched. Secret variable values are never searched.

### Check

`apinata check [collection]` looks for problems without sending anything or running a
script. It looks in the keychain only to see whether a secret has a value, and never
prints one; `--no-keychain` skips that, as it does for `run`. It reports a `{{name}}` that no scope defines, a
body file or client certificate that does not exist, a script that does not
parse, a file that failed to load, a request that cannot be built, and two
siblings that share a `seq`. Every environment is checked unless `--env NAME`
picks one. `--env-var KEY=VALUE` and `--global-env-var KEY=VALUE` (or a bare `KEY`, read
from the process environment) define names
the way `apinata run` does; the global environment is not read from disk, so
pass its names with `--global-env-var`. Secret values are replaced by a
placeholder and never printed. A script is only parsed, and the parse is
stricter than a run, so a script that runs today may still be reported.

```sh
apinata check
apinata check ./my-api --env dev
apinata check --json
```

Text output is one `severity: item [environment]: message` line per finding,
then a summary line; a clean collection prints only the summary. A request
that needs a run-time value (a prompt or a value set by a script) cannot be
fully resolved, and is counted in the summary rather than reported as an
error.

The exit codes are in the [table](#exit-codes). Warnings do not count as
errors.

`--json` prints a fixed document, whose `version` is `1`:

```json
{
  "version": 1,
  "summary": {
    "errors": 1,
    "warnings": 0,
    "items_checked": 3,
    "unresolved": [],
    "environments": ["dev"]
  },
  "findings": [
    {
      "item": "users/get.yml",
      "environment": null,
      "severity": "error",
      "kind": "undefined_variable",
      "message": "..."
    }
  ]
}
```

`item` is the path relative to the collection root with `/` separators, and
`environment` is `null` when the finding holds in every checked environment.
`severity` is `error` or `warning`. `kind` is one of `undefined_variable`,
`missing_file`, `script_syntax`, `unparsable_item`, `invalid_request`,
`duplicate_sequence` or `unclosed_placeholder` (a `{{name}` with no closing
`}}` in a request's url, headers, params, auth or body, which is sent as
literal text; the headers and auth of each `folder.yml` and of
`opencollection.yml` are scanned too, one finding per file and field).
`unresolved` lists the requests that could not be fully
resolved. Findings are sorted by item, kind, environment and message.

### List

`apinata ls [collection] [path]` prints the collection's folders and requests as a tree,
each request as its method, name and the path `apinata run` takes. `--tags`
adds the request tags and `--json` prints data. A folder whose name differs from
its directory shows both, and either one works in `path`. A file that fails to
load is listed last as `broken`, with the reason, and a folder whose `folder.yml`
fails shows the reason on its line; neither changes the exit code.

### Environments and secrets

A variable marked `secret: true` keeps no value in the file. The desktop app
stores secret values in the system keychain. `apinata run` and `apinata check` read
them from there, only when a secret needs a value; `--env-var` wins over a stored
value, and `--no-keychain` skips the keychain. `{{process.env.NAME}}` reads `NAME` from the shell environment,
with a `.env` file at the collection root on top; keep that `.env` out of git.

`apinata env ls` lists the environment names and `apinata env show <name>` lists one
environment's variables; a secret shows `<secret>` and never its value.
`apinata env set <name> KEY=VALUE` creates or changes a variable in an
environment file, and `apinata env unset <name> KEY` removes one; a second
identical `set` leaves the file byte for byte as it was. For a secret,
`apinata env set --secret <name> KEY` reads the value from stdin (one line) or a
hidden prompt, never the command line, stores it in the keychain and writes no
value to the file; `env unset --secret` removes a secret and its stored value.
`--no-keychain` skips the keychain, which makes a secret value impossible to store.

`apinata env new <name>` creates an empty environment, and `apinata env cp <name>`
copies one to `<name> copy`, with its stored secret values; a taken name gets a
suffix (`dev copy 2`) and the name used is printed (`--json` prints it as data).
`apinata env mv <from> <to>` renames an environment, moves its secret values and
re-points every environment that `extends` it; a name already taken is refused.
`apinata env rm <name>` deletes an environment and its secret values, and is
refused while another environment extends it, naming each one. With
`--no-keychain`, `cp`, `mv` and `rm` leave the stored values alone and say which.

`apinata env orphans <name>` lists stored secrets the environment's file no longer
marks secret (`--json` prints them as data), and
`apinata env forget <name> <VARIABLE>...` deletes them after a confirmation (`--all`
takes every one, `--yes` skips the question for scripts). This cannot be undone,
and another git branch may still use the values. With `--global` both work on a
global environment. `apinata env orphans --all` looks across every environment and
marks stored secrets whose environment has no file, as after a deletion or a rename
outside the app. `--no-file` makes either action take such a name as stored, and
`--old-root <path>` does the same for a collection root that no longer exists, for
which the path must be given exactly as it was, resolved (`//`, a `.` and trailing
slashes are tidied, nothing else). Both ask first, and a removal under either warns that
the directory may only be elsewhere, for example on a drive that is not mounted. Neither
ever touches an environment that has a file, even one that failed to parse.
`apinata env orphans --old-roots` finds those paths for you: it needs no collection and
prints each root that no longer exists but still holds stored secrets, followed by the
`env orphans --all --old-root` command that lists them (`--json` prints the roots as
data). Values are never read, and a root it cannot be sure is gone is left out. A store
that cannot list every collection makes it fail rather than print nothing.

### Items and folders

`apinata item` edits the tree the way the sidebar does, naming items by their
collection-relative path (`users/get.yml`, `users`). `new <name>` creates a request
from the collection's presets (`--in <folder>`, `--kind`, `--method`, `--url`), `mkdir
<name>` creates a folder, `cp <path> <name>` copies a request, or a folder with
everything in it, beside itself, `rename <path> <name>` renames a request or folder
with its file, and `mv <path>` moves one `--into` a folder (`.` is the root) or
`--before` or `--after` a sibling, renumbering both places. `rm <path>` moves the item
to the system trash and fails when there is none, and `restore` puts back the newest item
`rm` (or the MCP `delete_item` tool) trashed in this collection, refusing when something
is at its path now. A name already taken in the folder
gets a number, as `Find 1.yml`. Each prints the item's new path; `--format json` prints
it as data.

`folder <path>` changes what a folder (or the collection, with `.`) gives every request
below it: `--header NAME=VALUE` and `--var NAME=VALUE` set an entry (the first of that
name, enabling it, else a new one; a typed variable keeps its type), `--unset-header` and
`--unset-var` remove every entry of a name and fail when there is none,
`--enable-header`, `--disable-header`, `--enable-var` and `--disable-var` switch every entry
of a name on or off (after the sets, so `--header K=V --disable-header K` adds a disabled
header), one of `--auth none|inherit`, `--bearer`, `--basic USER:PASS`, `--digest
USER:PASS`, `--api-key NAME=VALUE` (`--api-key-in header|query`), or `--auth-json JSON` or
`--auth-json-file FILE` for any mode, OAuth 2.0 included, in the shape MCP's `read_folder`
shows, sets the auth, and `--docs` or `--docs-file`
(`-` for standard input) sets the docs, blank text removing them. A folder without
`folder.yml` gets one. It prints `updated` or `unchanged` with the file; `--format
json` prints `path`, `file` and `changed`. Values are written into the file as given, so
keep a secret in an environment (`apinata env set --secret`) and refer to it as
`{{name}}`.

```sh
apinata item new Find --in users --method GET --url "{{baseUrl}}/users?q=1"
apinata item mv users/Find.yml --before users/get.yml
apinata item rm users/old.yml
apinata item folder users --header X-Team=core --unset-var oldHost --bearer "{{token}}"
```

### Host overrides

`apinata network hosts list` shows the collection's host overrides, which send a host
name to chosen addresses instead of its DNS answer, as `host -> address`. `set <host>
<address>` adds or replaces a row (`--disabled` keeps it unused) and `remove <host>`
deletes it. An address is an IP, optionally with a port, and several can be listed
separated by commas; a port counts only for a URL that names none. The name stays in
the URL and the `Host` header, and a send to an MCP agent is judged by the address it
is mapped to, so a mapping off loopback needs approval whatever the name is.

### Replace

`apinata replace [collection] <pattern> <replacement>` rewrites the matches in the same
fields, touching only the matched bytes of each file. `--dry-run` lists the
hits and writes nothing, `--regex` reads the pattern as a regular expression
(`$1` works in the replacement), `--case` matches letter case exactly, and
`--json` prints data. A changed script asks for trust again.

```sh
apinata replace --dry-run api.example.com api.staging.example.com
apinata replace api.example.com api.staging.example.com
```

## Command line

### Exit codes

Every command follows one convention. `0` is success. `1` means a result you
asked to judge failed (an assertion, a check finding). `2` means a request or
transport failure. `3` means a usage, input or setup error, including a
collection that does not open, an address that cannot be bound and an output
file that cannot be written. A failed write to standard output (a full disk,
`/dev/full`) also exits 3, with one `error:` line on stderr; a closed pipe, as
with `| head`, is not an error and exits 0 quietly. The one exception is
`self-update --check`, which exits 2 when an update is available. A dash means
the command never exits with that code.

| Command | 0 | 1 | 2 | 3 |
| --- | --- | --- | --- | --- |
| `run` | all passed | an assertion or test failed, or a reply matched `--fail-on-status` | a request errored (no response) | usage error, a collection or file that failed to load, a report file could not be written, or no request matched |
| `send` | the request was sent and passed | a reply matched `--fail-on-status` | the request errored (no response) | usage error, an unusable URL or curl command, or a file that could not be written |
| `check` | no error findings (warnings do not count) | at least one error finding | - | usage error, collection cannot be opened, unknown `--env`, or JSON output could not be written |
| `ls` | listed | - | - | usage or collection error, the path is not a folder, or JSON output could not be written |
| `search` | searched, with or without hits | - | - | usage or collection error, or JSON output could not be written |
| `replace` | replaced or previewed, with or without matches | - | - | usage or collection error, or an invalid regular expression |
| `env` | listed, created, copied, renamed, set or removed | - | - | usage or collection error, unknown environment, a name that is invalid or taken, an environment another extends, or a refused edit |
| `network` | listed, set or removed | - | - | usage or collection error, or a refused edit |
| `item` | created, copied, renamed, moved, removed, or a folder updated or left unchanged | - | - | usage or collection error, an unknown path, an invalid name, a move that is not allowed, no trash to remove into, a broken folder.yml, an unknown header or variable to unset, or a refused edit |
| `trust` | done | - | - | any failure |
| `gen` | printed | - | - | usage, collection or resolve error |
| `graphql` | printed or written | the server answered without a schema (an error status, not JSON, GraphQL errors) | the request or its token fetch errored (no response) | usage, collection or resolve error, not a GraphQL request, or the file could not be written |
| `git` | printed, or no git or no repository (a note on stderr) | - | - | git failed, the file is not a request file or is outside the collection, or a usage or collection error; for `clone`, a bad URL or folder name, or the folder exists |
| `import` | imported (notes go to stderr) | - | - | the import failed |
| `export` | exported (notes go to stderr) | - | - | the export failed |
| `convert` | converted, planned (`--dry-run`) or declined | - | - | the conversion failed |
| `mock` | stopped with Ctrl+C | - | - | the server could not start: usage error, collection does not open, address refused or cannot be bound |
| `mcp` | the client disconnected | - | the stdio transport failed | the server could not start: no collection to serve, a bad option or a setup failure |
| `self-update` | up to date, installed, declined or not replaceable | - | `--check` found an update | an error |
| `completions` | script printed | - | - | usage error, such as an unknown shell |

One more code exists. `130` is a `git clone` or `git branches --fetch`
stopped with Ctrl+C, and a `run --watch` stopped with Ctrl+C before any run
finished. A usage error from the argument parser (an unknown flag, a missing
value) exits 3 for every command, not clap's usual 2. `--help` and `--version`
exit 0.

### Shell completions

Print a shell completion script with `apinata completions <shell>` (bash, zsh,
fish, elvish or powershell). Save the output where your shell looks for
completions; these are examples, adjust the paths to your setup:

```sh
apinata completions bash > ~/.local/share/bash-completion/completions/apinata
apinata completions zsh > ~/.zfunc/_apinata
apinata completions fish > ~/.config/fish/completions/apinata.fish
apinata completions elvish >> ~/.config/elvish/rc.elv
apinata completions powershell >> $PROFILE
```

### Running requests

Run every request in a collection with one of its environments:

```sh
apinata run ./my-api --env staging
```

The path may also be a folder inside the collection or one request file.
`--format json` writes a machine-readable report, and `--bail` stops after the
first failed or errored request:

```sh
apinata run ./my-api/users --format json --bail
apinata run ./my-api/users/get-user.yml --env local
```

`--watch` runs once, then reruns with the same options whenever a collection file changes, until Ctrl+C. A run in flight always finishes, edits made meanwhile cause one rerun, and report files are overwritten each time. Ctrl+C exits with the last finished run's code, or 130 if none finished. It cannot be combined with `--format json`.

```sh
apinata run ./my-api --env local --watch
```

### Variables and environments

Supply secrets and override variables with `--env-var`, which may repeat. A secret
without one is read from the keychain; an unreachable keychain is reported in the
hint of a request that needs the secret, and stops no other request:

```sh
apinata run ./my-api --env staging --env-var token=abc123 --env-var region=eu
```

A bare `--env-var KEY` reads the value from the process environment variable
`KEY`, so a secret never lands in the shell history or the process list:

```sh
TOKEN=abc123 apinata run ./my-api --env staging --env-var TOKEN
```

An unset `KEY` is a usage error (exit 3) that names the variable. A variable that
is set but empty gives the empty string. A value read this way is treated as a
secret, even when the environment does not declare the name secret: reports, the
`--verbose` lines and shown responses replace it with a placeholder. `--global-env-var KEY`
works the same way.

Global environments hold variables shared by every collection. They live in
`global-environments/` of the profile directory (`APINATA_PROFILE_DIR`, else
the config directory), sit below collection variables, and are chosen with
`--global-env`; `--global-env-var` sets or overrides one of their variables:

```sh
apinata run ./my-api --global-env shared --global-env-var token=abc123
```

`--workspace <DIR>` adds the shared environments of a workspace folder (one
holding `workspace.yml`). Name one as `"shared (workspace)"`; a plain name
picks the profile's environment when it has one, else the workspace's. The
workspace is only read.

### Reports

The JSON report's `summary` counts passed requests two more ways, both subsets
of `passed`: `unverified` is the passed requests whose every assertion and test
was skipped, so nothing was actually checked, and `no_tests` is the passed
requests that have no assertion and no test at all. The two never overlap. The
text report shows them as `(U unverified, T no tests)` after the passed count,
and neither changes the exit code.

A reply such as 404 or 500 passes a request that has no assertions. To fail
on the status alone, add `--fail-on-status`: a final reply of 400 or above
counts as a failed request, so the exit code is 1 and the summary names the
status. `--fail-on-status=4xx` or `--fail-on-status=5xx` narrows it to one
class (the value needs the equals sign). A request that a retry turned into a
good reply is not failed, and the JSON and JUnit reports carry the reason
(`status_failure` in the JSON).

```sh
apinata run probe.yml --fail-on-status --body | jq .
```

`--reporter-json`, `--reporter-junit` and `--reporter-html` each write a report file and can be combined with any `--format`; the directory of each path must already exist. A path of `-` writes that report to stdout and moves the console report to stderr; only one reporter can use `-`, and it cannot be combined with `--format json`, `--body`, `--show-response` or `--watch`:

```sh
apinata run ./my-api --reporter-junit results.xml --reporter-html results.html
apinata run ./my-api --reporter-junit - > results.xml
```

### Sending one request

`apinata send` sends one request without a collection, from a URL and flags or
from a curl command, and prints the response body. It takes `-X`, `-H` and `-d`,
or `--curl` with the command as the argument or `-` to read it from stdin (not
both). Use `--env-var NAME=VALUE` for `{{variables}}`, `-i` for the status line
and headers, `-o` to write the body to a file, and the `run` options
`--fail-on-status`, `--retries`, `--timeout`, `--insecure` and the network
flags; the exit codes are `run`'s. Nothing is written unless you add
`--save PATH`, which also keeps the request as a file. PATH must be a new
`.yml` file inside a collection, so `apinata run PATH` can send it later.
`--save-example NAME` with `--save` also keeps the response as a saved example
in that file.

`apinata run REQUEST --save-example NAME` saves the response of one request as
a saved example in its file, where `apinata mock` serves it. Secret values the
request knows become their `{{placeholders}}`, cookies and credential headers
are masked, and a binary body holding a secret is left out; `--keep-secrets`
saves the response exactly as received instead.

```sh
apinata send https://api.example.com/users -X POST -H 'Content-Type: application/json' -d '{"name":"Ada"}'
pbpaste | apinata send --curl - --save ./my-api/create-user.yml
```

### Code generation

`apinata gen` prints one request as code for curl, HTTPie, Python requests,
JavaScript fetch, Node axios, Go, PowerShell, PHP, Rust, Java or C#, without
sending it:

```sh
apinata gen ./my-api/users/get-user.yml --target python-requests --env local
```

### GraphQL schema

`apinata graphql schema` fetches a GraphQL request's schema by introspection,
resolving the request as `run` would send it, and prints the answer as JSON or,
with `--format sdl`, as an SDL document. `--output` writes it to a file:

```sh
apinata graphql schema ./my-api/graphql/users.yml --env local --format sdl --output schema.graphql
```

### OAuth 2.0 and scripts

OAuth 2.0 requests with the client credentials or password grant fetch their token on the first run and reuse it from an encrypted cache while it is valid; `--refetch-tokens` fetches new ones:

```sh
apinata run ./my-api --env staging --refetch-tokens
```

Collection, folder and request scripts run by default, and their `test()` results
and `console` output appear in the report. `--no-scripts` turns them off, and
`--script-timeout <SECS>` sets how long one script phase may take (default 30):

```sh
apinata run ./my-api --no-scripts
```

Scripts can `require("crypto")` for Node's hashes and HMACs, random bytes, `pbkdf2` and `scrypt`, AES-CBC, AES-CTR and AES-GCM ciphers, RSA (PKCS#1 v1.5 and PSS), ECDSA (P-256, P-384 and P-521) and Ed25519 signatures, and `KeyObject`s made from PEM, DER or JWK keys (RSA, P-256, P-384, P-521 and Ed25519; `export({ format: "jwk" })` too). RSA-PSS keys open from PEM or DER, encrypted or not, and report their hash, MGF1 hash and salt length in `asymmetricKeyDetails`; they sign and verify only within those parameters, as in Node, and have no JWK form. Encrypted PKCS#8 private keys open with `passphrase`, if they use PBES2 with PBKDF2 (SHA-1 or SHA-224 to SHA-512) or scrypt and AES or 3DES (DES-EDE3-CBC); other schemes, including Camellia and legacy `Proc-Type` PEM, are refused by name. `export({ format: "pem", type: "pkcs8", cipher, passphrase })` writes encrypted PKCS#8 with PBES2, PBKDF2-HMAC-SHA256 and AES-128, AES-192 or AES-256 in CBC mode, as Node does; other ciphers are refused by name. Key derivation cannot be stopped by the script timeout once it starts, so it is capped: PBKDF2 takes at most 10,000,000 iterations, and iterations x ceil(key length / digest length) must be at most 16,777,216; scrypt needs N x r x p at most 33,554,432 and `maxmem` at most 256 MiB; derived keys are at most 65,536 bytes. An encrypted key whose file asks for more is refused before any derivation. RSA-PSS signatures made with a plain RSA key and no `saltLength` use a salt as long as the digest; RSA-PSS keys salt as Node does. Other members throw an error that names them, and `require("crypto-js")` covers more algorithms.

### Timeouts, streams and body size

`--timeout <SECS>` caps every request's timeout for the run. The lower of the
request's own `timeout` setting and the flag wins, so a request with a shorter
timeout of its own keeps it, and a request with none gets the cap. A request that
hits the cap fails with an error that names the run-wide timeout, and the run exits
with code 2:

```sh
apinata run ./my-api --timeout 10
```

An event-stream response (`text/event-stream`) is read as it arrives and shown as received.
`--stream-max-time <SECS>` (default 30) and `--stream-max-events <N>` (default 10000) bound it,
and 0 means no limit. Reaching a bound is a normal end: the run continues, a note goes to stderr,
and assertions run on what arrived. The JSON report gains `streamEnd` (`closed`, `time_limit`,
`event_limit`, `idle_timeout`, `body_cap` or `error`) for a streamed response:

```sh
apinata run ./my-api --stream-max-time 5 --stream-max-events 100 --format json
```

A response body is cut after `--max-body-bytes <BYTES>` (default 10 MiB, 0 means no limit); a
cut body prints one note on stderr and assertions run on what was kept. In the desktop app the
notice for a cut response offers `Download full response`: it sends the request again (its
scripts run again, and a method other than GET, OPTIONS or HEAD asks first) and saves the whole
body to a file you pick. Cancel stops it and removes the partial file.

### Reading responses

To read a response, as with `curl -i`, add `-i` (`--show-response`): each response prints its
status line, headers, a blank line and the body on stdout, and the verdicts move to stderr. On a
terminal JSON is indented; piped, the bytes are left as received. `--body` prints only the
bodies, so it pipes into `jq`; with several requests the bodies follow one another unseparated.
`-o <PATH>` writes the body bytes exactly as received to a file (the last request's), or one
`NNN-<request>.body` per request when the path is an existing directory or ends in `/`. The file
is complete even past `--max-body-bytes` (stdout stays capped), and a failed transfer leaves no file. `-o -` is not stdout; it names a file called `-`. A
binary body never reaches a terminal; a size line stands in for it. Secret values are scrubbed
from what prints, but not from `-o` files. With `--format json`, `-i` adds a `response` object
(`status`, `headers`, `body`, `body_size`, `truncated`) to each result instead. Exit codes do not
change (see [exit codes](#exit-codes)).

`--filter <EXPR>` narrows what `--body` or `-i` prints (also on `apinata send`): a JSONPath
(RFC 9535) for a JSON response, an XPath 1.0 for an XML one, chosen by the content type. A path
that can match only one value, such as `$[0].name`, prints that value; any other JSONPath prints
the matches as an array (`[]` for none), and an XPath prints one node per line. The filter runs
after secrets are scrubbed and leaves `-o` files whole. A bad expression, or `--filter` with
`--format json`, exits 3 before any request; a body that is neither JSON nor XML fails that
request's output with exit 3:

```sh
apinata run ./my-api/users/list.yml --body --filter '$[0].name'
apinata run ./my-api/users/get.yml --body | jq .id
apinata run ./my-api/users/get.yml -i
apinata run ./my-api/avatar.yml -o avatar.png
```

## Mock server

`apinata mock` serves the saved examples of every request in a collection over
HTTP, so a front end can be built before the real API exists:

```sh
apinata mock ./my-api --env staging
apinata mock ./my-api --port 4010 --delay 200
```

The first line on stdout is `Listening on http://127.0.0.1:<port>`. The server
listens on loopback only, unless `--allow-non-loopback` is given with `--bind`.

When a request matches a route, the example is chosen by the
`x-mock-response-name` or `x-mock-response-code` header, then by the query
parameters recorded in the example, then the first example is used. A request
that matches nothing answers 404 with a JSON trace of what was tried.

A route with path parameters fills them into the example: for `/users/:id`, each
`{{id}}` in the example's body and header values becomes the matched segment
(percent-decoded); when a route repeats a name, the first segment's value is
used. Other names stay as written, and environment variables are never filled
in, so secrets cannot leak into a response. Binary bodies are served unchanged.

Examples are read once at start, so restart the server to pick up edits. Press
ctrl-c to stop it.

## Desktop app

Start `apinata-app`, or pass it one or more collection folders to open. From its menu or command palette you can Open collection,
New collection, Import collection and Import environment. Secret values you
enter are kept in the system keychain, never in the collection files. Check for
updates looks for a newer release.

New collection fills the folder you choose when it is empty, or holds only
`.git`, `.gitignore`, `.gitattributes` or `.DS_Store` (what a fresh repository or a
file manager leaves behind). A folder with any other entry is refused. The CLI
never fills an existing folder: `import collection` makes a new folder named
after the collection inside `--into`, and `convert --out` and `git clone` need a
folder that does not exist yet.

In the Import dialog, Update refreshes a collection from a newer OpenAPI link, shows a preview of what changes and can move requests whose operation vanished to the trash. Open tabs show the result at once.

Clone repository fetches a collection from a git URL into a folder you pick, and Switch branch (the chevron beside the branch name in the title bar) moves a cloned collection to another remote branch.

## Importing and exporting

Collections come in from Postman v2.0 and v2.1, OpenAPI 3 and Swagger 2.0,
Insomnia v4 and v5, Hoppscotch and HAR 1.2 captures. The format is detected,
or name it with `--from`:

```sh
apinata import collection postman.json --into ./collections
```

Bring environments along with `--env`, and for OpenAPI choose folders by path
segment instead of by tag with `--group-by path`:

```sh
apinata import collection openapi.yaml --into ./collections --group-by path
apinata import collection postman.json --into ./collections --env staging.postman_environment.json
```

When an OpenAPI description changes, `--update` refreshes a collection
imported from it: fields you have not edited are rewritten, new operations are
added, and requests whose operation vanished are listed as orphaned. `--prune`
also removes the orphans that are still as generated, and `--dry-run` only
reports. A request you restore after a prune is yours from then on: later updates
leave it alone and never delete or overwrite it. To track it again, delete it and
update from the description. `--into` is then the collection's own directory:

```sh
apinata import collection openapi.yaml --into ./collections/Shop --update --dry-run
```

To keep several descriptions in one collection, import each into its own folder
with `--folder`. The folder is created when missing and must not hold requests
yet, no environments are written, and `--update --folder` refreshes just that
folder later:

```sh
apinata import collection openapi.yaml --into ./my-api --folder Shop
apinata import collection openapi.yaml --into ./my-api --folder Shop --update
```

Environments from Postman and Hoppscotch go into an existing collection:

```sh
apinata import environment staging.json --collection ./my-api
```

One curl command becomes a request. Give it a file, or leave the file out to
read stdin; in the app, paste the command into the URL bar instead:

```sh
apinata import request command.txt --from curl --collection ./my-api
```

Anything that did not carry over as is gets a note on stderr. Credentials in a
HAR capture become valueless secret variables of a `Capture` environment.

A collection also goes out as a Postman v2.1 file, with `--out -` for stdout.
`--env-dir` writes each environment beside it with secret values left empty,
and whatever the file cannot carry, such as assertions, is noted on stderr:

```sh
apinata export ./my-api --format postman --out my-api.postman_collection.json --env-dir ./envs
```

In the app, "Export environments…" in the environment editor's "…" menu or the
command palette writes the same files to a folder you pick, and the global
environments dialog has the same button for the global set. Saved files are
exported, secret values stay empty, and a file of the same name is replaced.

`--format openapi` writes an OpenAPI 3.1 description instead, with environments
as servers, so it takes no `--env-dir`:

```sh
apinata export ./my-api --format openapi --out my-api.openapi.json
```

With `--format openapi`, an output file ending `.yaml` or `.yml` gets YAML with
the same keys in the same order; any other name, and `-` for stdout, gets JSON.
`--format openapi-yaml` writes YAML whatever the name. In the app, name the file
`.yaml` in the save dialog:

```sh
apinata export ./my-api --format openapi --out my-api.openapi.yaml
```

`--format html` writes one self-contained documentation page that opens
offline, without environments or credentials from auth:

```sh
apinata export ./my-api --format html --out my-api.docs.html
```

## Legacy .bru collections

A collection in the legacy .bru format converts to YAML, into a new folder by
default, leaving the original untouched. The desktop app offers the same when
you open one:

```sh
apinata convert ./old-api --out ./my-api
```

`--in-place` converts where the collection is and deletes the .bru files only
after every YAML file is written; `--dry-run` lists what would change and
writes nothing.

## MCP server

`apinata mcp` serves a collection to an AI agent over stdin and stdout:

```sh
apinata mcp --collection ./my-api
```

A client config looks like this:

```json
{"command": "apinata", "args": ["mcp", "--collection", "/path/to/my-api"]}
```

The agent can read, search, create, edit, move and delete requests, folders and
environments, import a request, send a request, run a folder, list and clear
cookies, list the trusted hosts, and read the git status and one file's
diff since the last commit. It can also find and replace text across the
requests with `replace`, which only previews the hits until `dry_run` is set
to false.

`send_request` takes `body_filter`, a JSONPath for a JSON response or an XPath for an XML one,
and returns only the matched text, so an agent can read a large list in parts. `total_bytes`
stays the whole body's size, `truncated` says whether the matches were cut to `max_body_bytes`,
and a bad expression is an error that sends nothing.

The agent can check the collection with `check_collection`, which returns the same
report as `apinata check --json` (undefined variables, missing files, script syntax
errors) and sends nothing; `env` limits it to one environment.

`set_runtime_variable` gives a session variable a value, so the next
`send_request` or `run_folder` resolves `{{name}}` to it, and
`clear_runtime_variables` removes some or all of them. They touch no file and no
secret, and their results never repeat a value.

The agent can write a request as code with `generate_code`, in any
target `apinata gen` knows. Secret values come out as `<secret>`, and nothing
is sent.

`graphql_schema` fetches a GraphQL request's schema by introspection, as
`apinata graphql schema` does, and returns it as SDL or as the JSON answer. It
sends like `send_request`, so a host other than a local one needs the same
approval.

The agent can also start the same mock server `apinata mock` runs from the
collection's saved examples with `mock_start`, read what it was asked with
`mock_hits`, and stop it with `mock_stop`. The mock always listens on
`127.0.0.1`, only one runs per session, and it stops when the session ends.
Starting it needs no approval because nothing leaves your machine.
`save_example` keeps the last response `send_request` got for a request as a
saved example, with every secret value the request knew replaced by its
placeholder, and `read_request` lists a request's examples.

The server also lists the collection's requests, folders and environments as MCP
resources (`apinata://<collection>/<path>`, read-only JSON, secret values never
shown) and offers three prompts: `write_tests`, `document_folder` and
`explain_failure`. Prompts and resources never send anything.

Clients can subscribe to a resource with `resources/subscribe` and stop with
`resources/unsubscribe`. When a file changes outside the server, such as an
edit in your editor or a `git pull`, a subscribed resource sends
`notifications/resources/updated`, and an added or removed request, folder or
environment sends `notifications/resources/list_changed`. A tool call that
writes collection files also sends `list_changed`; the read-only tools never
do. Change notifications are debounced, so a burst of edits arrives as one
after about 250 ms.

The agent may edit collection files freely, since git tracks them. Sending to a
host other than `localhost` or a loopback address needs approval: the request waits for you
to approve it in the desktop app, or you can trust the host beforehand. Denied
hosts are refused outright.

```sh
apinata trust add api.example.com --collection ./my-api
apinata trust list --collection ./my-api
```

`apinata trust list` prints the trusted hosts, then the denied ones;
`apinata trust deny` and `apinata trust remove` change the lists.

## Updates

```sh
apinata self-update --check
apinata self-update
```

`--check` only reports, and exits 2 when an update is available (see the [exit codes](#exit-codes)). In the app,
use Check for updates. The updater replaces a standalone CLI binary, an
AppImage, a macOS `.app` or the Windows installer's install. It leaves a `.deb`
install to the package manager and the CLI that ships with the app to the
app's own update, and says why. A build without an update source never checks;
it says "Update checks are not configured for this build".

## Release files

Every file in a release keeps its version in the name. The download links in
the [README](README.md#download-and-install) are copies of the app files under
fixed names, and they are not in `SHA256SUMS`, so check the versioned files.

Each release holds these files, where `<v>` is the version:

| Platform | Desktop app | Command-line runner |
| --- | --- | --- |
| Linux x86_64 | `apinata-app_<v>_amd64.deb`, `apinata-app_<v>_x86_64.AppImage` | `apinata-<v>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `apinata-app_<v>_arm64.deb`, `apinata-app_<v>_aarch64.AppImage` | `apinata-<v>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple silicon | `Apinata_<v>_aarch64.dmg` | `apinata-<v>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `Apinata_<v>_x86_64.dmg` | `apinata-<v>-x86_64-apple-darwin.tar.gz` |
| Windows x64 | `apinata-app_<v>_x64-setup.exe` | `apinata-<v>-x86_64-pc-windows-msvc.zip` |

The `.deb` installs both `/usr/bin/apinata` and `/usr/bin/apinata-app`. A
release also carries the raw CLI binaries and `Apinata_<v>_<arch>.app.tar.gz`
files, which the updater downloads. One `SHA256SUMS` file covers every asset;
check your downloads with:

```sh
sha256sum -c --ignore-missing SHA256SUMS
```

Files built without the project's signing certificate carry `-unsigned` in
their names, such as `apinata-app_<v>_x64-setup-unsigned.exe`. The macOS and
Windows signing notes are in the [README](README.md#signing-and-checksums) and
[RELEASING.md](RELEASING.md).

## Building and packaging

Building needs Rust 1.99 or newer and, on Linux, the packages listed in
[Build from source](README.md#build-from-source). Run the checks (format, lint,
tests), then build:

```sh
make check
cargo build --release
```

The binaries land in `target/release/`. To build the Linux `.deb`, AppImage
and CLI tarball into `target/dist/linux/`, install cargo-packager and run:

```sh
cargo install --locked cargo-packager --version 0.11.8
make package
```

`make demo` builds the app and opens a sample collection that talks to
jsonplaceholder.typicode.com:

```sh
make demo
```

It runs on a copy under `~/.cache/apinata-demo` with its own profile there, so
your real settings and the repository stay untouched. The collection copy is
reset on every run; the demo profile is kept until you delete that folder.

`make install` builds and installs the app and the CLI for the current user,
and `make uninstall` removes them; where the files go is in
[Build from source](README.md#build-from-source).
