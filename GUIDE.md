# Apinata guide

The full reference for Apinata's collections, command line, mock server, desktop
app, importers, MCP server and releases. For what Apinata is, how to install it
and a quick start, see the [README](README.md).

## Contents

- [Working with collections](#working-with-collections)
  - [Creating a collection](#creating-a-collection)
  - [Naming the collection](#naming-the-collection)
  - [Git](#git)
  - [Search](#search)
  - [Check](#check)
  - [List](#list)
  - [Environments and secrets](#environments-and-secrets)
  - [Items and folders](#items-and-folders)
  - [Host overrides](#host-overrides)
  - [Unix sockets](#unix-sockets)
  - [Replace](#replace)
- [Command line](#command-line)
  - [Exit codes](#exit-codes)
  - [Shell completions](#shell-completions)
  - [Running requests](#running-requests)
  - [Variables and environments](#variables-and-environments)
  - [Reports](#reports)
  - [Running in CI](#running-in-ci)
  - [Sending one request](#sending-one-request)
  - [Code generation](#code-generation)
  - [GraphQL schema](#graphql-schema)
  - [GraphQL queries](#graphql-queries)
  - [OAuth 2.0 and scripts](#oauth-20-and-scripts)
  - [JWT Bearer auth](#jwt-bearer-auth)
  - [Team servers](#team-servers)
  - [Timeouts, streams and body size](#timeouts-streams-and-body-size)
  - [Reading responses](#reading-responses)
- [Mock server](#mock-server)
- [Desktop app](#desktop-app)
  - [Team workspaces](#team-workspaces)
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

### Creating a collection

`apinata init [DIR] [--name NAME]` creates an empty collection in `DIR`, the
working directory by default, and creates `DIR` when it is missing. The name
defaults to the directory's name. It writes the same `opencollection.yml` the
desktop app's New collection does, and like it fills only an empty directory
(see [Desktop app](#desktop-app)). A directory that already holds
`opencollection.yml` is refused.

```sh
apinata init ./my-api
apinata init --name "My API"
```

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
| `env`, `trust`, `git switch`, `git commit`, `git pull`, `git push` | none | `--collection <DIR>` |

`env`, `trust`, `git switch`, `git commit`, `git pull` and `git push` take
their own arguments first, so a leading collection would be ambiguous and only
the flag exists. In `ls`, one argument
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
`apinata git switch` checks one out. `apinata git pull` fetches and
fast-forwards the current branch, and `apinata git push` pushes it to its
upstream; neither merges, rebases or forces. A new branch with no upstream yet
takes `--set-upstream <REMOTE>`, which publishes it under its own name on that
remote and tracks it from then on:

```sh
apinata git push --set-upstream origin --collection ./my-api
```

`apinata git commit -m <MESSAGE>` commits locally and prints the new short id
and subject (`--json` prints `{"short_id", "subject"}`). Name files or folders
relative to the working directory to stage them first, or pass `--all` for the
whole collection; with neither, it commits what is already staged. The
repository's git hooks run, and nothing is pushed.

```sh
apinata git commit -m "Fix the users URL" users/get-user.yml
apinata git commit -m "Update everything" --all
```

The desktop app marks changed requests in
the sidebar and tabs, and lists the changes with their diffs in a Changes
dialog. Its Discard button, Delete, or Discard Changes in a file's right-click
menu throws away that file's changes after asking: a file in the last commit
goes back to it, a new file is removed, and your version goes to the system
trash. Undo on the notice that follows puts your version back. When there is no
trash, or the file's staged version differs from the one on disk, the question
says the discard cannot be undone and no Undo is offered. A file with unsaved
edits in a tab or with conflicts is left alone, and so is a new or renamed file when
there is no trash to keep it in.

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

`apinata env extends <name> <parent>` makes an environment inherit the variables of
another, and `apinata env extends <name> --clear` removes the parent. A parent that
is the environment itself, has no file, or leads back to it (`environment
inheritance cycle: a -> b -> a`) is refused with exit code 3 and nothing changes.
`env show` prints the parent as an `extends:` line, or an `extends` field with
`--json`.

`--global` makes `ls`, `show`, `set`, `unset`, `new`, `cp`, `mv`, `rm` and `extends` work on the
[global environments](#variables-and-environments) instead of a collection's, with the same
output, exit codes and secret handling; no collection is needed, and `--global` with
`--collection` is a usage error:

```sh
apinata env new shared --global
apinata env set shared host=api.example.com --global
```

`apinata env orphans <name>` lists stored secrets the environment's file no longer
marks secret (`--json` prints them as data), and
`apinata env forget <name> <VARIABLE>...` deletes them after a confirmation (`--all`
takes every one, `--yes` skips the question for scripts). This cannot be undone,
and another git branch may still use the values. With `--global` both work on a
global environment, and `--collection` is then refused. `apinata env orphans --all` looks across every environment and
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

When a pull renames an environment or a secret variable, the stored value stays under the
old name, and `apinata git pull` says so with a hint.
`apinata env renames --since <REV>` lists each such secret from that commit to `HEAD`
as `dev / token → staging / token` with a status: `ready`, `ambiguous` (more than one new
name, or a guess; every candidate is listed) or `taken` (a different value is already
stored under the new name). Without `--since` it starts at `ORIG_HEAD`, which a pull sets
but a merge, rebase or reset also moves, so `--since` with the first id the pull printed is
the safe choice. `--move` moves every ready one: it writes the new name, reads it back and
only then deletes the old one, and lists the others as skipped, never overwriting a value.
`--pick dev/token=staging/api_token` (repeatable) settles an ambiguous one. A secret moved
here is no longer offered in the app, and the reverse. Values are never printed.

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
is at its path now. `restore --pruned` instead puts back every request the newest
`apinata import --update --prune` removed, and `--batch <ID>` picks an older prune by the id
a refusal lists; if any of them clashes with what is there now, nothing is restored.
`scrub-userinfo` lists the saved examples whose recorded URL holds a username or password
and, with `--write`, removes them from those URLs; the requests' own URLs stay as they are.
A name already taken in the folder
gets a number, as `Find 1.yml`. Each prints the item's new path; `--format json` prints
it as data.

`rename-example <path> <example> <name>` renames a request's saved example, and
`rm-example <path> <example>` removes one, so `apinata mock` stops serving it. Both find
the example by its name and change nothing, exiting 3, when no example has it, when more
than one does (rename one in the app or edit the file first), or, for a rename, when
another example already has the new name. They print what changed; `--format json`
prints `path` with the new `name` or the `removed` one.

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

`apinata item request <path>` edits one request the way MCP's `update_request` does, and
the file it writes is the same. `--name`, `--method` and `--url` set those fields (a new
URL derives the params again). The list flags work as in `item folder`: `--header`,
`--var`, `--query` (query params) and `--form` (form-urlencoded or multipart text fields)
take `NAME=VALUE`, each with `--unset-`, `--enable-` and `--disable-` forms, and
`--assert 'EXPR=OP VALUE'` with `--unset-assert`, `--enable-assert` and
`--disable-assert` edits assertions. `--path-param NAME=VALUE` sets a path param the URL
names. `--body-mode none|json|xml|text|sparql|form-urlencoded|multipart|file` switches
the body, `--body TEXT` or `--body-file FILE` (`-` for standard input) sets its text (a
GraphQL request's query, with `--graphql-variables`), `--form-file NAME=PATH` and
`--form-type NAME=TYPE` edit multipart files, and `--select-file PATH`, `--file-type
PATH=TYPE` and `--unset-file PATH` edit a file body. The auth flags, `--docs` and
`--docs-file` are `item folder`'s. `--timeout MS|unlimited|inherit`, `--encode-url`,
`--follow-redirects`, `--forward-auth` (each `true` or `false`) and `--max-redirects N`
change the settings, `--tag` and `--unset-tag` the tags, `--seq N` the position, and
`--sync-query url|params` settles a query that disagrees with the params. It prints
`updated` or `unchanged`; `--format json` prints `path` and `changed`. Values, `{{vars}}`
included, are written as given, with the same advice for secrets.

```sh
apinata item new Find --in users --method GET --url "{{baseUrl}}/users?q=1"
apinata item mv users/Find.yml --before users/get.yml
apinata item rm users/old.yml
apinata item rm-example users/get.yml "not found"
apinata item folder users --header X-Team=core --unset-var oldHost --bearer "{{token}}"
apinata item request users/Find.yml --method POST --body-mode json --body '{"q": "{{term}}"}'
```

### Host overrides

`apinata network hosts list` shows the collection's host overrides, which send a host
name to chosen addresses instead of its DNS answer, as `host -> address`. `set <host>
<address>` adds or replaces a row (`--disabled` keeps it unused) and `remove <host>`
deletes it. An address is an IP, optionally with a port, and several can be listed
separated by commas; a port counts only for a URL that names none. The name stays in
the URL and the `Host` header, and a send to an MCP agent is judged by the address it
is mapped to, so a mapping off loopback needs approval whatever the name is.

### Unix sockets

On Linux, macOS and other Unix systems, a request can go to a server listening on
a Unix socket. Write the socket path after `http://unix:`, then a colon and the
request path:

```text
http://unix:/var/run/docker.sock:/v1.47/containers/json?all=1
```

The request goes over that socket as if to `http://localhost`, and never
through a proxy. Special characters in the socket path, a colon among them, are
percent-encoded (`%3A`). The request path must start with `/`, the URL takes no
port, and HTTPS over a socket is not supported. A relative redirect stays on the
same socket; an absolute one goes where it points. On Windows a socket URL is
refused with an error, never sent to TCP `localhost`.

For an AI agent a socket is never local, whatever it serves: sending to one needs
approval, and it is trusted by its path, as `unix:/var/run/docker.sock` (see
[MCP server](#mcp-server)).

### Proxy and client certificates

`apinata network proxy show` prints the collection's proxy mode (`inherit`, `off` or
`custom`) and a custom proxy's fields. `proxy set` switches to a custom proxy with
`--protocol` (`http`, `https`, `socks4`, `socks5`), `--host`, `--port`, `--bypass`
(comma separated hosts) and `--username`; a flag left out keeps the value the file
already holds, and `--no-auth` drops the credentials. `proxy off` connects directly
and `proxy clear` goes back to the global setting; both keep the custom fields in the
file, as the app does.

`apinata network cert list` shows the client certificates as `domain -> kind files`.
`cert set <domain> --cert <file> --key <file>` (PEM) or `cert set <domain> --pfx <file>`
(PKCS#12) adds or replaces the domain's entry, `--disabled` keeps it unused, and
`cert remove <domain>` deletes it. A path inside the collection is stored relative to
it, and a relative path is read from the collection.

A PEM entry takes a certificate file and a key that is plain or encrypted PKCS#8. A
PKCS#12 entry (`.pfx` or `.p12`) is opened with its passphrase, which resolves
`{{variables}}` like any other value; both the AES-256 bundles current OpenSSL writes
and the older RC2 and 3DES bundles of Windows and browser exports are read. A wrong
passphrase, a file that is not PKCS#12 and a bundle without a private key each fail
the send with an error naming the file, never the passphrase.

A proxy password or certificate passphrase is never taken as a plain flag value, so it
stays out of shell history. `--password` and `--passphrase` accept only a
`{{variable}}` reference. `--password-stdin` and `--passphrase-stdin` read the value
from one line of stdin, or a hidden prompt on a terminal, store it in the collection's
`.env` (as `APINATA_PROXY_PASSWORD` or `APINATA_CERT_PASSPHRASE_<n>`, adding `.env` to
`.gitignore`) and write a `{{process.env.NAME}}` reference to the file. `apinata
network move-secrets` does the same for literal secrets already in the file. `show` and
`list` print references as written and never a literal value. These settings are CLI
and app only: the MCP server cannot change them.

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
| `run` | all passed | an assertion or test failed, or a reply matched `--fail-on-status` | a request errored | usage error, a collection or file that failed to load, a report file could not be written, or no request matched |
| `send` | the request was sent and passed | a reply matched `--fail-on-status` | the request errored | usage error, an unusable URL or curl command, or a file that could not be written |
| `check` | no error findings (warnings do not count) | at least one error finding | - | usage error, collection cannot be opened, unknown `--env`, or JSON output could not be written |
| `init` | created | - | - | the directory already holds a collection, is not empty, or could not be written |
| `ls` | listed | - | - | usage or collection error, the path is not a folder, or JSON output could not be written |
| `search` | searched, with or without hits | - | - | usage or collection error, or JSON output could not be written |
| `replace` | replaced or previewed, with or without matches | - | - | usage or collection error, or an invalid regular expression |
| `env` | listed, created, copied, renamed, set or removed, or every rename asked for moved | `renames --move` or `--pick` skipped an ambiguous or taken rename | - | usage or collection error, unknown environment, a name that is invalid or taken, an environment another extends, a refused edit, an unknown `--since`, or a store error |
| `network` | listed, set or removed | - | - | usage or collection error, or a refused edit |
| `item` | created, copied, renamed, moved, removed, or a folder updated or left unchanged | - | - | usage or collection error, an unknown path, an invalid name, a move that is not allowed, no trash to remove into, a broken folder.yml, an unknown header or variable to unset, or a refused edit |
| `oauth2` | shown, cleared, or nothing to clear | - | - | usage or collection error, an unknown path, auth that is not OAuth 2.0, or settings that do not resolve |
| `team` | signed in, signed out, or every sign-in shown is valid | signed out, expired, or the server refused the sign-in | the server could not be reached or its answer was not understood | usage or local error, such as `--no-keychain` with `sign-in` or `sign-out`, `APINATA_TEAM_TOKEN` without `APINATA_TEAM_SERVER` or a malformed value in either, `sign-out` for the server in `APINATA_TEAM_SERVER`, a team file or keychain failure, or another Apinata window or command changing the team file for over two seconds |
| `trust` | done | - | - | any failure |
| `gen` | printed | - | - | usage, collection or resolve error |
| `graphql` | printed or written | the server answered without a schema (an error status, not JSON, GraphQL errors), a syntax error, a validation error, or `fmt --check` found a change | the request or its token fetch errored (no response) | usage, collection or resolve error, not a GraphQL request, the file could not be written, or an unreadable query or schema file |
| `git` | printed, or no git or no repository (a note on stderr; `commit`, `pull` and `push` fail instead) | - | - | git failed (for `commit`, also a hook refusing or nothing staged; for `commit`, `pull` and `push`, also no git or repository), the file is not a request file or is outside the collection, or a usage or collection error; for `clone`, a bad URL or folder name, or the folder exists |
| `import` | imported (notes go to stderr) | - | - | the import failed |
| `export` | exported (notes go to stderr) | - | - | the export failed |
| `convert` | converted, planned (`--dry-run`) or declined | - | - | the conversion failed |
| `mock` | stopped with Ctrl+C | - | - | the server could not start: usage error, collection does not open, address refused or cannot be bound |
| `mcp` | the client disconnected | - | the stdio transport failed | the server could not start: no collection to serve, a bad option or a setup failure |
| `self-update` | up to date, installed, declined or not replaceable | - | `--check` found an update | an error |
| `completions` | script printed | - | - | usage error, such as an unknown shell |

One more code exists. `130` is a `git clone`, `git branches --fetch`,
`git pull` or `git push` stopped with Ctrl+C, a `team sign-in` stopped with
Ctrl+C, and a `run --watch` stopped with Ctrl+C before any run finished. A usage error from the argument parser (an unknown flag, a missing
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

Every `run` flag, by what it changes. `apinata run --help` lists the same flags with their
defaults. `-q` (`--quiet`) works on every command and hides notices that are not errors, such
as the warning that a collection's network settings changed since the last run.

Environments and variables (see [Variables and environments](#variables-and-environments)):

| Flag | Meaning |
| --- | --- |
| `--collection <DIR>` | The collection, the same as the PATH argument. |
| `--env <NAME>` | The environment to use, by file name without `.yml`. Without it, the collection's default environment (set in its presets) is used when it exists. |
| `--no-env` | Use no environment, even when the collection names a default one. |
| `--env-file <PATH>` | Read the environment from a file anywhere on disk: an OpenCollection environment or a Postman environment export. Cannot be combined with `--env` or `--no-env`. |
| `--env-var KEY[=VALUE]` | Set or override a variable; a bare `KEY` reads the process environment. Repeatable. |
| `--no-keychain` | Never read secrets from the keychain; each one then needs `--env-var`. |
| `--global-env <NAME>` | The global environment to use. |
| `--global-env-var KEY[=VALUE]` | Set or override a global environment variable. Repeatable. |
| `--workspace <DIR>` | Also offer a workspace folder's environments to `--global-env`. |

Filtering:

| Flag | Meaning |
| --- | --- |
| `--tags <TAGS>` | Run only the requests that have at least one of these comma-separated tags. |
| `--exclude-tags <TAGS>` | Skip the requests that have any of these tags, even when `--tags` picks them. |
| `--bail` | Stop after the first failed or errored request. |

Data-driven runs:

| Flag | Meaning |
| --- | --- |
| `--csv-file-path <PATH>` | Run once per row of a CSV file whose first row names the columns; each column is a `{{variable}}` for its row. |
| `--json-file-path <PATH>` | Run once per object of a JSON array file; each key is a `{{variable}}`. Not with `--csv-file-path`. |
| `--iterations <N>` | Run the collection N times; with a data file, use at most its first N rows. |
| `--delay <MS>` | Wait this many milliseconds between requests (default 0). |

Retries and status:

| Flag | Meaning |
| --- | --- |
| `--retries <N>` | Send again, up to N more times, when the send fails in transit (timeout, refused or reset connection) or the reply is 502, 503 or 504. Scripts and assertions run once, on the last attempt. |
| `--retry-delay <MS>` | Wait this long before each retry; a `Retry-After` header can lengthen it, up to 60 seconds. |
| `--retry-unsafe` | Also retry POST and PATCH, which a repeat could apply twice. Without it only GET, HEAD, OPTIONS, PUT, DELETE and TRACE retry. |
| `--fail-on-status[=4xx\|5xx]` | Fail a request whose final status is 400 or above (or in the class given), even without assertions. |

Cookies:

| Flag | Meaning |
| --- | --- |
| `--disable-cookies` | Neither store nor send cookies between the requests of this run. |
| `--cookie-jar <FILE>` | Load cookies from this JSON file before the run and save them back after it, so later runs keep them. A missing file starts empty; the file is plain text, so keep it out of git. Not with `--disable-cookies`. |

Network:

| Flag | Meaning |
| --- | --- |
| `--insecure` | Accept TLS certificates that fail verification. |
| `--cacert <FILE>` | Also trust the CA certificates in this PEM file. |
| `--ignore-truststore` | With `--cacert`, trust only that file and not the system's certificates. |
| `--client-cert-config <FILE>` | Add client certificates from a JSON file (`{"enabled": true, "certs": [...]}`). |
| `--noproxy` | Connect directly, ignoring the collection's proxy and the system's. |
| `--proxy-pac <URL\|PATH>` | Pick the proxy for requests that use the global proxy setting from this proxy auto-config (PAC) file. Not with `--noproxy`. |
| `--http-version <VERSION>` | `1.1` (the default) sends HTTP/1.1 only, `auto` uses HTTP/2 over HTTPS when the server offers it, and `2-prior-knowledge` uses HTTP/2 without asking, plain `http` too. |

Scripts, tokens and limits (see [OAuth 2.0 and scripts](#oauth-20-and-scripts) and
[Timeouts, streams and body size](#timeouts-streams-and-body-size)):

| Flag | Meaning |
| --- | --- |
| `--no-scripts` | Do not run scripts; their tests count as skipped. |
| `--script-timeout <SECS>` | The longest one script phase may take (default 30). |
| `--refetch-tokens` | Fetch new OAuth 2.0 tokens instead of using cached ones. |
| `--timeout <SECS>` | Cap every request's timeout for this run. |
| `--stream-max-time <SECS>` | End an event stream after this long (default 30, 0 for no limit). |
| `--stream-max-events <N>` | End an event stream after about this many events (default 10000, 0 for no limit). |
| `--max-body-bytes <BYTES>` | Cut a response body after this many bytes (default 10 MiB, 0 for no limit). |

Output (see [Reports](#reports) and [Reading responses](#reading-responses)):

| Flag | Meaning |
| --- | --- |
| `--format text\|json` | The report written to stdout. |
| `--reporter-json <PATH>` | Also write the JSON report to a file, or `-` for stdout. |
| `--reporter-junit <PATH>` | Also write a JUnit XML report. |
| `--reporter-html <PATH>` | Also write a self-contained HTML report. |
| `--report-omit-headers` | Leave every header out of the JSON report's `response` objects. |
| `--report-omit-header <NAME>` | Leave the header `NAME` out of them; repeatable, any case. |
| `--report-omit-body` | Leave the body out of them; `body_size` stays. |
| `--verbose` | Print one line per network exchange to stderr: method, URL, status or error, the address reached and the time, plus each cookie set. Never a body or a header. |
| `-i`, `--show-response` | Print each response's status line, headers and body. |
| `--body` | Print only each response body. |
| `--filter <EXPR>` | Narrow what `--body` or `-i` prints with a JSONPath or XPath. |
| `-o`, `--output <PATH>` | Write the response body bytes to a file, or one file per request in a directory. |
| `--har <FILE>` | Also record every exchange the run sent as one HAR 1.2 log, with secret values and credentials replaced by `[redacted]`. |
| `--save-example <NAME>` | Save the response of the one request run as a saved example in its file. |
| `--keep-secrets` | With `--save-example`, keep the response exactly as received. |
| `--watch` | Rerun whenever a collection file changes, until Ctrl+C. |

```sh
apinata run ./my-api --env staging --tags smoke --exclude-tags slow --retries 2 --retry-delay 500
apinata run ./my-api/users --csv-file-path users.csv --delay 200 --har run.har
apinata run ./my-api --cookie-jar .cookies.json --http-version auto --cacert ca.pem
```

### Variables and environments

Supply secrets and override variables with `--env-var`, which may repeat. A secret
without one is read from the keychain; an unreachable keychain is reported in the
hint of a request that needs the secret, and stops no other request:

```sh
apinata run ./my-api --env staging --env-var token=abc123 --env-var region=eu
```

Without `--env`, `run`, `gen`, `graphql schema`, `mock`, `oauth2` and `mcp` use the
default environment the collection's presets name, as the app does when it opens the
collection. A default that names no existing environment prints a warning and the
command runs with no environment. `--env` picks another one, and `--no-env` runs with
none.

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

`run --env-file PATH` takes the environment from a file outside the collection, such
as one a CI job writes, without copying it in:

```sh
apinata run ./my-api --env-file ./ci.json --env-var TOKEN
```

The file is an OpenCollection environment (a `variables` list, named by its file
name) or a Postman environment export (named by its `name`); the kind is told from
the content, not the extension. It replaces the collection's environments, the preset
default included, so it cannot be combined with `--env` or `--no-env`; `--env-var`
still wins over its values. A file that uses `extends` is refused, since only a
collection holds the environments it could inherit from. Secrets are never read
from the keychain for such a file, even when it shares a name with one of the
collection's environments: a `secret: true` variable needs `--env-var`, and a
Postman `secret` value is used as given and hidden like any other secret. OAuth 2.0
tokens fetched in such a run are kept in memory only. A key the Postman reader has to
rename, such as one with a space, is printed as a warning. The path must be a regular
file, so a pipe or `<(...)` is refused. With `--watch`, each rerun reads the file
again, but editing it does not start a rerun, since only the collection is watched.
A file that is missing, unreadable or of another shape, including a Hoppscotch
export, is a usage error (exit 3) that names the path.

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

### Running in CI

A CI job installs the command-line runner from a release, checks it against
the release's `SHA256SUMS` and runs the collection from the repository. Pin
the version: the runner has no version-free download name, and a pinned
version keeps the job repeatable. The Linux runners are built on Ubuntu 24.04
and need a system at least that new. The commands below install Linux x86_64;
the other targets are in [Release files](#release-files).

```sh
v=0.2.0 t=x86_64-unknown-linux-gnu
base=https://github.com/PhongGuy/apinata/releases/download/v$v
curl -fsSL -O "$base/apinata-$v-$t.tar.gz" -O "$base/SHA256SUMS"
sha256sum -c --ignore-missing SHA256SUMS
tar -xzf "apinata-$v-$t.tar.gz"
install -D -m 0755 "apinata-$v-$t/apinata" "$HOME/.local/bin/apinata"
```

`sha256sum` fails, and so stops the job, when the download does not match.
Store each secret in the CI's secret store and pass it by name with a bare
`--env-var KEY`, so its value never appears in the job's command line or log
(see [Variables and environments](#variables-and-environments)); an unset
`KEY` fails the run with exit code 3. `--no-keychain` skips the keychain, which
a CI machine does not have. The run's exit code fails the job (see
[Exit codes](#exit-codes)): `1` for a failed assertion or test, `2` for a
request that errored, `3` for a setup error such as a missing environment.
When a job fails, read the failing request in the log or the JUnit report, and
rerun it locally with `apinata run <request file> --env <name> --verbose`.

In GitHub Actions, the collection in `api/` with a `staging` environment and
an `API_TOKEN` repository secret:

```yaml
name: API tests
on: [push, pull_request]

permissions:
  contents: read

jobs:
  api:
    runs-on: ubuntu-24.04
    env:
      APINATA_VERSION: 0.2.0
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Install apinata
        run: |
          t=x86_64-unknown-linux-gnu
          base=https://github.com/PhongGuy/apinata/releases/download/v$APINATA_VERSION
          curl -fsSL -O "$base/apinata-$APINATA_VERSION-$t.tar.gz" -O "$base/SHA256SUMS"
          sha256sum -c --ignore-missing SHA256SUMS
          tar -xzf "apinata-$APINATA_VERSION-$t.tar.gz"
          install -D -m 0755 "apinata-$APINATA_VERSION-$t/apinata" "$HOME/.local/bin/apinata"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Run the collection
        env:
          API_TOKEN: ${{ secrets.API_TOKEN }}
        run: |
          mkdir -p reports
          apinata run api --env staging --env-var API_TOKEN --no-keychain --reporter-junit reports/junit.xml --reporter-html reports/report.html
      - name: Upload the reports
        if: ${{ !cancelled() }}
        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: api-reports
          path: reports/
```

The upload runs after a failed run too, so the reports of a red job are kept.
GitHub Actions has no built-in JUnit view: the job log shows the text report,
and the uploaded `report.html` is the readable one. A JUnit reporting action can
turn `junit.xml` into a check, but it needs the `checks: write` permission.

In GitLab CI, with `API_TOKEN` as a masked CI/CD variable:

```yaml
api-tests:
  image: ubuntu:24.04
  variables:
    APINATA_VERSION: "0.2.0"
  before_script:
    - apt-get update -qq
    - apt-get install -y -qq --no-install-recommends ca-certificates curl
    - t=x86_64-unknown-linux-gnu
    - base=https://github.com/PhongGuy/apinata/releases/download/v$APINATA_VERSION
    - curl -fsSL -O "$base/apinata-$APINATA_VERSION-$t.tar.gz" -O "$base/SHA256SUMS"
    - sha256sum -c --ignore-missing SHA256SUMS
    - tar -xzf "apinata-$APINATA_VERSION-$t.tar.gz"
    - install -m 0755 "apinata-$APINATA_VERSION-$t/apinata" /usr/local/bin/apinata
  script:
    - mkdir -p reports
    - apinata run api --env staging --env-var API_TOKEN --no-keychain --reporter-junit reports/junit.xml
  artifacts:
    when: always
    reports:
      junit: reports/junit.xml
```

GitLab reads `artifacts:reports:junit` into the pipeline's Tests tab and the
merge request's test summary, failed runs included.

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

For a [Unix socket](#unix-sockets) URL, the curl target, like the app's Copy as
curl, names the socket with `--unix-socket` and puts the request path on a
`http://localhost` URL.

### GraphQL schema

`apinata graphql schema` fetches a GraphQL request's schema by introspection,
resolving the request as `run` would send it, and prints the answer as JSON or,
with `--format sdl`, as an SDL document. `--output` writes it to a file:

```sh
apinata graphql schema ./my-api/graphql/users.yml --env local --format sdl --output schema.graphql
```

### GraphQL queries

`apinata graphql fmt` formats a query file (`.graphql` or `.gql`) or a GraphQL
request's query in place; `--check` writes nothing and exits 1 if the file
would change. `{{variables}}` in the query are kept as written:

```sh
apinata graphql fmt ./my-api/graphql/users.yml --check
```

`apinata graphql validate` checks a query and prints one line per finding:
its position, `error`, `warning` or `info`, and the message. Without
`--schema-file` only syntax is checked; with it, given as SDL or an
introspection JSON answer, fields, arguments and types are checked too, and
uses of deprecated fields and enum values are warned about. It exits 1 when
any finding is an error:

```sh
apinata graphql validate ./my-api/graphql/users.yml --schema-file schema.graphql
```

### OAuth 2.0 and scripts

OAuth 2.0 requests with the client credentials or password grant fetch their token on the first run and reuse it from an encrypted cache while it is valid; `--refetch-tokens` fetches new ones:

```sh
apinata run ./my-api --env staging --refetch-tokens
```

`apinata oauth2 status` lists the cached tokens of a collection, one line each: where the auth
is defined (a request, a folder or the collection), the grant, when the token expires or that it
has, its scope and whether a refresh token came with it. Token values are never shown.
`apinata oauth2 clear` forgets them, so the next run fetches new ones; clearing nothing is not an
error. Both take a request or folder path to act on just that item's auth, `--env` and
`--global-env` for tokens fetched with an environment or a global environment, and
`--format json`:

```sh
apinata oauth2 status --collection ./my-api --env staging
apinata oauth2 clear users/get.yml --collection ./my-api --env staging
```

Collection, folder and request scripts run by default, and their `test()` results
and `console` output appear in the report. `--no-scripts` turns them off, and
`--script-timeout <SECS>` sets how long one script phase may take (default 30):

```sh
apinata run ./my-api --no-scripts
```

Scripts can `require("crypto")` for Node's hashes and HMACs, random bytes, `pbkdf2` and `scrypt`, AES-CBC, AES-CTR and AES-GCM ciphers, RSA (PKCS#1 v1.5 and PSS), ECDSA (P-256, P-384 and P-521) and Ed25519 signatures, and `KeyObject`s made from PEM, DER or JWK keys (RSA, P-256, P-384, P-521 and Ed25519; `export({ format: "jwk" })` too). RSA-PSS keys open from PEM or DER, encrypted or not, and report their hash, MGF1 hash and salt length in `asymmetricKeyDetails`; they sign and verify only within those parameters, as in Node, and have no JWK form. Encrypted PKCS#8 private keys open with `passphrase`, if they use PBES2 with PBKDF2 (SHA-1 or SHA-224 to SHA-512) or scrypt and AES or 3DES (DES-EDE3-CBC); other schemes, including Camellia and legacy `Proc-Type` PEM, are refused by name. `export({ format: "pem", type: "pkcs8", cipher, passphrase })` writes encrypted PKCS#8 with PBES2, PBKDF2-HMAC-SHA256 and AES-128, AES-192 or AES-256 in CBC mode, as Node does; other ciphers are refused by name. Key derivation cannot be stopped by the script timeout once it starts, so it is capped: PBKDF2 takes at most 10,000,000 iterations, and iterations x ceil(key length / digest length) must be at most 16,777,216; scrypt needs N x r x p at most 33,554,432 and `maxmem` at most 256 MiB; derived keys are at most 65,536 bytes. An encrypted key whose file asks for more is refused before any derivation. RSA-PSS signatures made with a plain RSA key and no `saltLength` use a salt as long as the digest; RSA-PSS keys salt as Node does. Other members throw an error that names them, and `require("crypto-js")` covers more algorithms.

### JWT Bearer auth

A request, folder or collection with `type: jwt` auth signs a fresh token on every send and
sends it as a bearer token:

```yaml
auth:
  type: jwt
  algorithm: RS256            # HS, RS and PS 256 to 512, ES256, ES384 or ES512
  secret: "{{jwt_secret}}"    # HS only; secretBase64: true decodes it first
  privateKey: {type: text, value: "{{jwt_key}}"}   # RS, PS and ES; type: file reads a PEM file
  headers: '{"kid":"k1"}'     # optional extra header keys
  payload: '{"sub":"{{user}}"}'
  expiresIn: 300              # optional; adds iat and exp when the payload has neither
  placement: header           # header or query
  headerPrefix: Bearer
  queryParamKey: token
```

Variables are filled into every field before anything is signed, so `apinata check` names each
one a request needs. The payload must be a JSON object once filled. The token's `alg` always
comes from `algorithm`. Keys are PEM: PKCS#8, PKCS#1 or SEC1, and a `type: file` path is
relative to the collection root. In the header the token replaces any `Authorization` header the
request has; in the query it replaces a parameter with the same name. An empty secret or key, a
key that does not match the algorithm, an unknown algorithm or a payload that is not a JSON
object stops the request before it is sent, and the error never shows the payload, key, secret
or the value an `algorithm` variable held. The token, the secret and the key are hidden in
history, reports and the Timeline.

### Team servers

`apinata team` signs in to a self-hosted team server the way the desktop app
does, and both share the sign-in: the token is kept in the system keychain and
the account in the same team file, so signing in from either works for both.

```sh
printf '%s\n' "$TEAM_PASSWORD" | apinata team sign-in https://team.example.com --username ada
apinata team sign-in https://team.example.com --sso
apinata team status
apinata team status https://team.example.com --format json
apinata team sign-out https://team.example.com
```

The password is read from stdin, or from a hidden prompt in a terminal, and is
never taken from the command line or an environment variable. `--sso` prints
the address to open in a browser and the local port it waits on, opens the
address in your default browser, then waits up to five minutes. If no browser
can be started it says "could not open a browser; open the address above" and
keeps waiting. It only prints the address, opening nothing, with
`--no-browser`, when `CI` is set, on Linux and BSD when neither `DISPLAY` nor
`WAYLAND_DISPLAY` is set, and on macOS and Windows when `SSH_CONNECTION` is
set; an empty variable counts as unset. Ctrl+C at the password prompt or
during the sign-in cancels it, exits 130 and keeps nothing. Signing in
on another machine over SSH, forward the printed port first, for example
`ssh -L 53682:127.0.0.1:53682 build-box`, then open the address in your local
browser.

When the browser cannot reach the waiting port, for example because it runs on
another machine, paste the address the browser ended on (it starts with
`http://127.0.0.1:`, or `http://[::1]:` on a machine without IPv4 loopback, and
ends in `/callback?...`) into the terminal within a minute and press Enter; the
first valid answer, pasted or from the browser, signs you in. A pasted address
must be this sign-in's own: the same scheme, host, port and path, and the state
it was started with. Otherwise the command says why, for example "that address
belongs to another sign-in", and keeps waiting. Lines over 8 KiB are refused,
and the end of input stops only the pasting, not the wait. Run as a background
job (`&`), the command does not offer the paste and waits for the browser only.
The pasted text is shown as you type; the code in it is single-use, expires
after a minute and only works with a key that never leaves the command.

`status` checks each kept sign-in with its server and exits 1 when one is
signed out or has expired; an expired sign-in is reported without asking the
server, as `the sign-in to <server> has expired; run "apinata team sign-in
<server>"`. As in the app, a token the server no longer accepts is forgotten,
while the account stays listed as signed out, and the name the server gives
is kept. `sign-out` revokes the token on the server, then forgets it.
With `--no-keychain`, `sign-in` and `sign-out` refuse to run, so a token is
never forgotten here while it still works on the server, and `status` shows
only the kept account and its expiry. Tokens and token ids are never printed,
and hidden characters in what the server sends are shown as `\u{...}` escapes.

A CI job has no keychain, so it passes a team token in two environment
variables instead:

```sh
export APINATA_TEAM_SERVER=https://team.example.com
export APINATA_TEAM_TOKEN="$CI_TEAM_TOKEN"   # from the CI system's secret store
apinata team status --no-keychain
```

The token goes only to the server whose address equals
`APINATA_TEAM_SERVER`, and for that server it wins over the keychain, which is
not opened for it; other servers use their kept sign-ins as before. `status`
needs no sign-in on the machine, reports the user as `(token from
APINATA_TEAM_TOKEN)` (`"source": "environment"` in JSON), and a token the server
refuses exits 1 without changing the keychain or the team file. `sign-out`
refuses for that server, since the token comes from the environment, and
`sign-in` ignores both variables. An empty variable counts as unset. A token
without `APINATA_TEAM_SERVER`, a value that is not a token or an address, or a
plain `http://` address to another machine, exits 3 before any request, naming
the variable but never its value; use `https://`. The token is kept out of
`{{process.env.*}}` and scripts in any letter case, so
`{{process.env.APINATA_TEAM_TOKEN}}` is an undefined variable, and out of every
program the CLI starts, git included. The desktop app never reads either
variable, but if it is started from a shell that holds the token, the browser
or system app it opens a link or file with can inherit it.

To get a token for CI, have a server admin create a dedicated user with only
the grants the job needs, sign in as that user, and mint a personal access
token with the server's `POST /api/v1/tokens` (`{"name": "ci", "expires_at":
...}`). The token is shown once; store it in the CI system's secrets, and
revoke it on the server when the job no longer needs it.

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

That JSON also goes to a `--reporter-json` file, so a CI artifact can carry what the server
sent. `--report-omit-headers` removes the `headers` key, `--report-omit-header <NAME>` (repeatable,
matched in any case) removes one header, and `--report-omit-body` removes the `body` key while
`body_size` and `truncated` stay. Secrets are still scrubbed from what is left. The JUnit and HTML
reports never hold headers or bodies, and the plain `-i` and `--body` output is unchanged. An empty
or malformed header name exits 3. Assertion values, console lines and error messages can still
quote parts of a body:

```sh
apinata run ./my-api -i --format json --reporter-json report.json --report-omit-body --report-omit-header x-request-id
```

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
Ctrl+C to stop it. The desktop app's mock server reloads edits on its own (see
[Desktop app](#desktop-app)).

## Desktop app

Start `apinata-app`, or pass it one or more collection folders to open. From its menu or command palette you can Open collection,
New collection, Import collection and Import environment. Secret values you
enter are kept in the system keychain, never in the collection files. Check for
updates looks for a newer release.

New collection fills the folder you choose when it is empty, or holds only
`.git`, `.gitignore`, `.gitattributes` or `.DS_Store` (what a fresh repository or a
file manager leaves behind). A folder with any other entry is refused.
`apinata init` follows the same rule and writes the same files. Otherwise the
CLI never fills an existing folder: `import collection` makes a new folder named
after the collection inside `--into`, and `convert --out` and `git clone` need a
folder that does not exist yet.

In the Import dialog, Update refreshes a collection from a newer OpenAPI link, shows a preview of what changes and can move requests whose operation vanished to the trash. Open tabs show the result at once.

Clone repository fetches a collection from a git URL into a folder you pick, and Switch branch (the chevron beside the branch name in the title bar) moves a cloned collection to another remote branch.
The same dialog has Pull, which fetches and fast-forwards the current branch,
and Push, which pushes it to its upstream; the command palette has them as Pull
from remote and Push to remote. A branch that is not on the remote yet offers
Publish to followed by the remote's name instead.

When a pull renames a secret variable, its stored value stays under the old
name, and a notice offers Review (a team collection lists them in its Team sync
dialog). Check for renamed secrets in the command palette looks again. The Renamed secrets dialog lists each one: Move carries
the value to the new name, Enter again lets you type it afresh, and Dismiss
drops a row whose new name already holds a different value.

Check collection, in a collection's sidebar menu or the command palette, runs
the same checks as [`apinata check`](#check) and lists the findings, errors
first. Choosing a finding opens the tab for its file, or shows the file in the
sidebar when no tab can show it. Re-run checks again after you fix something.

Start mock server, in a collection's sidebar menu or the command palette, or
the Mock button in the status bar, serves that collection's saved examples on
127.0.0.1, as [`apinata mock`](#mock-server) does. While it runs, the status
bar shows its address with a copy button and the number of examples. Click the
number to see the routes served and, under Received, the latest calls, newest
first, each with the example it got; choosing a call opens that example. The
server picks up saved edits by itself, and Stop mock server (or the Mock
button again) stops it.

Run collection and Run folder open a runner tab. Its Cookies choice says which
cookie jar the run uses: Shared jar (the default) uses the jar every
send shares, Empty jar gives the run a jar of its own that starts empty, and Copy
of the shared jar gives it its own copy. With a jar of its own, Keep cookies
after the run puts the cookies the run set or changed into the shared jar;
cookies it deleted stay there. Keep variable values set by the run is on by
default; off, the run works on a copy of the variables and drops the values it
set when it ends. A collection or folder runner remembers these choices.

The bottom dock has Console, Network and Variables tabs. Variables lists the
variables the active request sees, grouped by the scope that defines them, after
the names a send would refuse as undefined. A value a higher scope overrides is
dimmed, struck through and labelled with the scope that wins, and a secret shows
a mask. Runtime and environment values, and undefined
names, are edited in place; any other row opens its scope's own editor. Show
variables in the command palette, or the variables button in the title bar,
opens the tab. While the request uses an undefined variable, a warning badge
before Send shows how many; its tooltip names them, and clicking it opens the
Variables tab.

The response status line shows the status, the time, the size and the address
reached, each with a hover card. The status card says what the code means, the
time card breaks the time down, and the size card splits headers from body. The
address card shows the connection: the remote address (or the proxy's), the
local address and the HTTP version, and over TLS the TLS version and the
server certificate's subject, issuer and expiry date. A response with no
recorded connection, such as one restored from history, shows only the address.

### Team workspaces

Join team server in the command palette signs in to a self-hosted
[Potluck](https://github.com/PhongGuy/potluck) server. Enter its address; when
you leave the field or press Enter, the app asks the server how it takes
sign-ins. With a password, enter your username and password. When the server
offers single sign-on, a Sign in with button naming its provider appears
under the password fields, or replaces them when the server takes single sign-on
only. It opens the sign-in page in your browser and shows its address with
Copy URL, in case the browser did not open; the dialog waits up to five minutes
for the browser to come back. The password is sent once and never stored; the
sign-in is kept in the system keychain. The Team workspaces dialog then opens,
and Team workspaces in the command palette brings it back. It lists each
server's workspaces: Open clones one and opens it as a collection, a server
admin can create a new one, and Sign out leaves that server.

A team collection syncs by itself: saved changes are committed and pushed, and
your teammates' changes are pulled in, so its Switch branch dialog has no Pull
or Push. The status bar shows each team collection's sync state, and clicking
it opens the Team sync dialog, which says what holds the sync up and how to
clear it. When two people changed the same request, Compare shows both sides,
and Keep theirs or Use mine settles it. Offline, changes are committed and pushed
once the server is back.

When the server refuses a push, the dialog says "The server refused the push"
and lists each finding with its file and git ref; when it found secrets, it says
so. Remove the secrets or fix what it names, save, then choose Retry.

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

A collection set up for OpenAPI sync in another app records its spec's link or
file in `opencollection.yml`. Leave the description out and `--update` uses that
source, grouped as recorded, and names it on stderr (a link is fetched like any
other; in the app, Update fills in the recorded source). The recorded source
updates the whole collection, so updating a folder still needs the description. The first update of such a collection does not know
which values were generated, so it adds new operations, keeps every request that
already differs from the description as it is and lists it under "differs", and
removes nothing. From then on updates work as above.

```sh
apinata import collection --into ./collections/Shop --update
```

An update never overwrites a request you edited, or one listed under "differs".
To take the description's version anyway, name the request with `--take-spec`,
using the path the update printed. It overwrites every field that differs from
the description; add `:FIELD,FIELD` (`target`, `headers`, `body`, `docs`,
`tags`, `vars`, `method`, `examples`) to take only those. The flag may be
repeated, works with `--dry-run`, and a path the update does not match stops it
before anything is written. Afterwards later updates treat the request as
generated again. In the app, the update preview has a Take from spec box on
each edited or differing request, and MCP's `import_collection` takes the same
paths as `take_spec`.

```sh
apinata import collection openapi.yaml --into ./collections/Shop --update --take-spec people/getUser.yml
apinata import collection openapi.yaml --into ./collections/Shop --update --take-spec people/getUser.yml:docs
```

To keep several descriptions in one collection, import each into its own folder
with `--folder`. The folder is created when missing and must not hold requests
yet. The imported collection's own auth, headers, variables, scripts and docs go
on that folder, its environments are added to the collection (one whose name is
taken gets a new name and a note), and `--update --folder` refreshes just that
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

Scripts are translated to Postman's script API on the way out, each call
becoming its `pm` counterpart. A call with no Postman translation is kept as
written and noted. `--keep-scripts` exports every script
unchanged instead; in the app, "Export collection to Postman, keeping scripts as
written…" in the app menu or a collection's sidebar menu does the same:

```sh
apinata export ./my-api --format postman --keep-scripts --out my-api.postman_collection.json
```

In the app, "Export all to Postman…" in the environment editor's "…" menu, or
"Export all environments to Postman…" in the command palette, writes the same
files to a folder you pick, and the global environments dialog has the same
button for the global set. Saved files are
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
offline. Environments are left out unless you pass `--include-environments`,
and credentials from auth are never written:

```sh
apinata export ./my-api --format html --out my-api.docs.html
```

`--include-scripts` adds each request's assertions and scripts as escaped code
(never run), and `--include-environments` adds a section with every
environment's variables as written, unresolved; secret variables show a mask
instead of their value.

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

Tools that resolve variables use the `environment` they are given, else the server's
`--env`, else the collection's preset default environment while it exists; `--no-env`
starts the server with no default environment.

A client config looks like this:

```json
{"command": "apinata", "args": ["mcp", "--collection", "/path/to/my-api"]}
```

The agent can read, search, create, edit, move and delete requests, folders and
environments (and the global environments, with the `_global_environment`
tools under the same secret rules), import a request, send a request, run a folder, list and clear
cookies, set or delete one cookie, list the trusted hosts, and read the git status and one file's
diff since the last commit, and commit collection changes locally with
`git_commit` (the repository's hooks run; nothing is pushed). It can also find and replace text across the
requests with `replace`, which only previews the hits until `dry_run` is set
to false.

These are all the tools the server offers.

Reading the collection:

- `get_collection`: the collection's name, root, environments, default environment and any files that failed to load.
- `list_items`: the folders and requests as a tree, one folder or one page at a time on a large collection.
- `search`: find text in requests' names, URLs, headers, bodies, scripts and docs, never in secret values.
- `check_collection`: the `apinata check` report, without sending anything.
- `read_request`: one request file, with its saved examples.
- `read_folder`: the headers, auth, variables and docs a folder (or the collection root) shares.
- `list_trusted_hosts`: the hosts this collection may reach without approval, and the denied ones.

Changing requests and folders:

- `update_request` and `update_folder`: change only the fields sent.
- `create_request` and `create_folder`: add a new request or folder.
- `import_request`: create a request from a curl command, without sending it.
- `import_collection`: add a Postman, OpenAPI, Swagger, Insomnia, Hoppscotch or HAR collection as a new folder, or refresh an earlier OpenAPI import with `update`.
- `clone_request` and `clone_folder`: copy an item beside itself under a new name.
- `rename_item` and `move_item`: rename an item, or move it into a folder or next to a sibling.
- `delete_item`: move an item to the trash.
- `restore_item`: put back the item the newest delete moved to the trash.
- `restore_pruned`: put back the requests one pruning `import_collection` update moved to the trash.
- `replace`: find and replace text across the requests, previewing until `dry_run` is false.

Sending, examples and exports:

- `send_request`: send one request and return its response.
- `run_folder`: run every request under a folder.
- `generate_code`: write a request as code, sending nothing.
- `oauth2_token`: show, fetch, refresh or clear a request's or folder's OAuth 2.0 token, never returning the token.
- `save_example`, `rename_example` and `delete_example`: keep the last response as a saved example, or rename or remove one.
- `scrub_example_userinfo`: remove usernames and passwords from the URLs saved examples recorded, previewing until `write` is true.
- `export_har`: the last exchange of a request as HAR, with secrets and credentials replaced by `[redacted]`.
- `export_openapi` and `export_postman`: the whole collection as OpenAPI 3.1 or Postman v2.1 text, without secret values.
- `graphql_schema`, `graphql_format` and `graphql_validate`: fetch a GraphQL schema, and format or check a query.
- `mock_start`, `mock_hits` and `mock_stop`: run the mock server on `127.0.0.1`, read what it answered, stop it.

Session state:

- `list_cookies`, `set_cookie`, `delete_cookie` and `clear_cookies`: read and change this session's cookie jar.
- `list_runtime_variables`, `set_runtime_variable` and `clear_runtime_variables`: read and change the variables responses captured this session.

Environments:

- `read_environment`: one environment's variables, secrets by name only.
- `create_environment`, `duplicate_environment`, `rename_environment`, `update_environment` and `delete_environment`: manage the collection's environments.
- `list_global_environments`, `read_global_environment`, `create_global_environment`, `duplicate_global_environment`, `rename_global_environment` and `update_global_environment`: the same for the global environments, which cannot be deleted from MCP.
- `list_secret_renames`: stored secrets that renames since `ORIG_HEAD` left under an old name, names only; `apinata env renames --move` or the app moves them.

Git:

- `git_status` and `git_diff`: the changed files, and one file's changed sections and line diff since the last commit.
- `git_commit`: commit collection changes locally.
- `git_branches`, `git_switch_branch`, `git_pull` and `git_push`: list and switch branches, and sync with the upstream.

The agent can also work with branches and the remote. `git_branches` lists the
local branches and the remote branches as of the last fetch, and never fetches.
`git_switch_branch` switches to a local branch, or to a remote one as
`<remote>/<name>`, and is refused while a tracked file has uncommitted changes.
Untracked files are carried over, unless the branch holds one of them; then the
switch is refused with the files named. `git_pull`
fast-forwards the current branch to its upstream, and `git_push` pushes it to
its existing upstream, never forced and never setting a new upstream. Pull and
push follow the same trust rule as sends: a remote on a host other than a local
path, a `file://` URL, `localhost` or a loopback address needs that host
trusted for the collection, and an untrusted one is refused before git runs and
waits for your approval in the desktop app.

The desktop app and the MCP server share a lock on the repository, `apinata.lock`
in its git dir. The app holds it while it pulls or switches branch, and every
MCP tool that writes collection files or git state (edits, imports, deletes and
restores, the collection's environments, `git_commit`, `git_switch_branch`,
`git_pull` and `git_push`) holds it for the whole call, so neither lands half
way through the other. The lock belongs to the repository, so two collections in
one repository share it, and a linked worktree has its own. A tool that finds
the lock taken waits up to 2 seconds, then changes nothing and returns an error
result (`isError` true) with this text:

```text
a pull or branch switch is running in the app, or another tool is changing this collection; try again
```

Read-only tools never wait for the lock, and outside a git repository there is none.

`send_request` takes `body_filter`, a JSONPath for a JSON response or an XPath for an XML one,
and returns only the matched text, so an agent can read a large list in parts. `total_bytes`
stays the whole body's size, `truncated` says whether the matches were cut to `max_body_bytes`,
and a bad expression is an error that sends nothing.

The agent can check the collection with `check_collection`, which returns the same
report as `apinata check --json` (undefined variables, missing files, script syntax
errors) and sends nothing; `env` limits it to one environment.

`set_cookie` stores one cookie as if a response from its `url` had set it, so later sends to
that domain and path carry it; setting one contacts no host. `domain` widens it to a parent
domain of the URL's host, and a domain the URL could not set is refused. `delete_cookie` takes
the `domain`, `path` and `name` a `list_cookies` entry shows and says whether anything matched.
Neither tool ever repeats a cookie's value.

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

`graphql_format` and `graphql_validate` format and check a GraphQL query, as
`apinata graphql fmt` and `apinata graphql validate` do. Each takes either a
GraphQL request's `path` or raw `query` text. `graphql_format` rewrites a
request's saved query in place, only when it changes; `graphql_validate` checks
syntax, and with a `schema` (SDL or an introspection answer) the query against
it. Nothing is sent and no approval is needed.

The agent can also start the same mock server `apinata mock` runs from the
collection's saved examples with `mock_start`, read what it was asked with
`mock_hits`, and stop it with `mock_stop`. The mock always listens on
`127.0.0.1`, only one runs per session, and it stops when the session ends.
Starting it needs no approval because nothing leaves your machine.
`save_example` keeps the last response `send_request` got for a request as a
saved example, with every secret value the request knew replaced by its
placeholder, and `read_request` lists a request's examples. `rename_example` and
`delete_example` rename or remove a saved example by name, refusing as `apinata item
rename-example` and `rm-example` do.

`update_environment` and `update_global_environment` take an `extends` field: a
name sets the environment's parent, `null` clears it, and leaving it out keeps it.
They refuse the same parents `apinata env extends` does, with the same message, and
the read tools report the parent as `extends`.

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

The agent may edit collection files freely, since git tracks them. Global
environments live outside git, so the agent cannot delete one; delete it with
`apinata env rm --global` or in the app. Sending to a
host other than `localhost` or a loopback address, or to any
[Unix socket](#unix-sockets), needs approval: the request waits for you to
approve it in the desktop app, or you can trust the host beforehand. Denied
hosts are refused outright.

```sh
apinata trust add api.example.com --collection ./my-api
apinata trust list --collection ./my-api
```

`apinata trust list` prints the trusted hosts, then the denied ones;
`apinata trust deny` and `apinata trust remove` change the lists. Each takes a host or a
pasted `http` or `https` URL, which keeps only its host, or a socket as `unix:/path/to.sock`. In the desktop app the same lists
are on a collection's AI access settings tab, and Preferences has an AI agents (MCP) section
with the connect command.

The desktop app and the MCP server run a collection's scripts only once you approve them.
`apinata trust scripts` prints every script text in the collection not yet approved (scripts,
script variables and assertion expressions), asks once, and on `y` approves exactly those and
allows scripts for the collection; `--yes` skips the question. `apinata run` does not ask.

```sh
apinata trust scripts --collection ./my-api
```

## Updates

```sh
apinata self-update --check
apinata self-update
```

`--check` only reports, and exits 2 when an update is available (see the [exit codes](#exit-codes)). It lists
what changed in every release newer than yours, newest first, grouped into new
features, fixes and so on. When the list does not reach back to your version,
it points at the download page for the older ones; when the notes cannot be
loaded it says so and still offers the update. `--check --format json` prints
`current`, `available`, `notes` (the releases, or null), `older_not_listed`
(true when the list stops short of your version, so older notes are on the
download page) and `notes_error`.
In the app, use Check for updates; View notes in the update notice shows the
same lists, and installing stays a separate step. The updater replaces a standalone CLI binary, an
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

To check that this project's release workflow built a file, run
`gh attestation verify <file> --repo PhongGuy/apinata`. The macOS and
Windows signing notes are in the [README](README.md#signing-and-checksums) and
[RELEASING.md](RELEASING.md).

## Building and packaging

Building needs Rust 1.99 or newer and, on Linux, the packages listed in
[Build from source](CONTRIBUTING.md#build-from-source). Run the checks (format, lint,
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
[Build from source](CONTRIBUTING.md#build-from-source).
