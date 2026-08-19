# RFC 0013 — What the console owes a reader looking at one package

| Field      | Value                                                                                          |
| ---------- | ---------------------------------------------------------------------------------------------- |
| Status     | **Accepted** — every phase below is built on `feat/readme` and unmerged; §12 records what each one changed |
| Author     | batleforc                                                                                       |
| Co-author  | —                                                                                               |
| Created    | 2026-08-18                                                                                      |
| Supersedes | —                                                                                               |
| Touches    | `crates/config`, `crates/core` (readme render/sanitise, hot config), `crates/web` (explore detail/fetch, README, images), `ui`, docs |

---

## 1. Summary

Eleven changes to the two pages a reader actually uses — the catalog and a
package's own page — plus three new configuration parameters and one refused
request. They are argued together because they are the same defect wearing
eleven costumes: **the console knew something and could not act on it, or acted
on it and could not say so.**

The catalog knew what you had searched for and lost it the moment you opened a
result, and the package page did the same thing one level down with the version
list: which versions you had narrowed it to, and which page of them you were on.
The package page knew which version it had selected and could not show
you which. It knew a package's README came from markdown and would not show you
the markdown. It rendered fenced code as grey text because the sanitiser had
erased the language on the way out. It offered a **Fetch this version** button to
a reader with no session, whom the endpoint would then refuse — and refused
nothing at all, because the endpoint did not check.

### Before / after

```text
# today
/packages?q=chalk         → search state is component state; Back gives an empty box
/packages/npm/chalk       → 896px of page on a 1920 screen; 44 rows, no filter, no pager
                            selected version marked with a 1.06:1 fill nobody can see
                            README: rendered only, code blocks grey
                            pre-releases mixed into the list, sometimes selected by default
POST …/fetch (no session) → 409 already-held  ← it had passed every gate

# with this RFC
/packages?registry=npm&q=chalk&sort=name      ← the search *is* the URL; Back restores it
/packages/npm/chalk?version=4.0.2&q=4.0&page=2 ← the list state is the URL too
                            full width; filter + pager; releases only until asked
                            selection on a crimson edge and raised ink
                            README: rendered ⇄ source, both highlighted by Shiki
POST …/fetch (no session) → 401 fetch.unauthenticated, before anything is fetched
```

---

## 2. Motivation

1. **A search that cannot be returned to is a search you do the twice.** The
   catalog held registry, query, scope, sort and page in component state and
   nowhere else, and the detail page's back button pushed
   `/packages?registry=…` — a URL the catalog never read. Opening the fifth
   result of a search and pressing Back gave page one with an empty box.

2. **A page that selects something must be able to point at it.** The version
   marker was `bg-muted/40`, which on this ground computes to about 1.06:1
   against the sheet: DESIGN.md's Undependable Fill Rule names exactly this, and
   the page was relying on it.

3. **The default selection announced the wrong thing.** It took the first row
   the endpoint returned, preferring what this instance holds — so an instance
   holding one release candidate opened on the candidate, and the README panel
   followed it. What a proxy should say first is what it *serves*, and that is
   the newest **release** it holds.

4. **The rendering of a README was the only rendering available**, though the
   endpoint has carried `source_text` since RFC 0007 §4.4. The console never
   asked for it: `format` defaults to `html`, and the parameter that would have
   returned both was never sent. The one reader who most needs the source — the
   one surprised by the rendering — had no way to see it.

5. **Fenced code arrived without its language.** `pulldown-cmark` writes
   `class="language-js"` and the sanitiser dropped it, because `class` is not in
   the attribute allow-list and only one class name (`readme-stripped-image`) was
   permitted. Measured on `chalk`: seven fenced blocks, every one anonymous by
   the time it reached the panel. Highlighting was not "not built" — it was
   unbuildable.

6. **`remote_images = "proxy"` was all or nothing.** A README that badges from
   `shields.io` and screenshots from a personal domain forced a choice between
   proxying both and chipping both. Operators asked for the middle, which is also
   the honest position: the badge hosts are known, the rest are whatever an
   author wrote.

7. **A pull required no session.** `POST …/{version}/fetch` admitted an
   anonymous caller. Measured against the running instance before the fix, an
   anonymous request answered `409 already-held` — it had passed visibility, the
   operator's `console_fetch` switch and the kind check, and was stopped only by
   the artifact happening to be there. The reasoning in the handler
   ("a download the caller could already run with `curl`") is right about
   *reading* and silent about the fact that a fetch **writes**: it fills the
   cache, spends bandwidth on both sides, extracts an SBOM, and files an audit
   row whose actor would read `anonymous`.

8. **A hundred and sixty-nine rows is not a list.** `@babel/plugin-transform-runtime`
   has 169 versions and the table drew all of them, above a page whose useful
   content sat higher up. The endpoint had built every one of those rows too —
   a vulnerability read and a licence read each — so the cost was paid twice
   over for a table showing 25.

---

## 3. Goals / non-goals

**Goals**

- A search survives opening a result and coming back — including the scroll
  offset.
- A version is a linkable destination; a link to one opens on it, on the page
  that holds it, whether or not it is a pre-release.
- The reader can see which version is selected without measuring a fill.
- A README can be read as rendered *or* as source, with code highlighted in both.
- An operator can name the hosts a README image may be proxied from.
- Causing this instance to fetch requires saying who you are.

**Non-goals**

- Anything about how a README is *stored* or extracted (RFC 0007 settled it).
- Client-side sanitisation. The boundary stays server-side, and this RFC does not
  add a second `v-html` — see §6.3, which is the whole argument for the Shiki
  approach it takes.
- A general download-token mechanism for clients that cannot authenticate; that
  is RFC 0012, and this one only refuses the anonymous case rather than
  replacing it.
- Making the catalog's own list paginate differently, or touching the admin
  package table.

---

## 4. User-facing design

### 4.1 The catalog and the URL

Registry, query, scope, sort and page serialise into the query string, defaults
omitted, written with `replace` rather than `push` — a keystroke is not a
destination, and pushing would make Back walk letter by letter out of the search.

`Back to catalog` calls `router.back()` when the previous entry *is* the catalog
(matched on the exact path, because a neighbouring package page also starts with
`/packages`), and pushes `/packages?registry=…` otherwise — a pasted link, a
refresh, a new tab. `scrollBehavior` returns `savedPosition` on a pop, which is
the part a rebuilt URL cannot give back.

#### How long a catalog page is

`20` was a literal in two places — a `serde` default on the endpoint and a
`const perPage` in the console — so the number could not be changed without a
rebuild and the two copies could disagree. It is `[limits].packages_per_page`
now, with the same two readings as its version-list sibling: the unasked-for
default *and* the ceiling.

Two keys rather than one, because the two lists are not the same question. A
catalog row is a name and a handful of counts, and 20 is a screenful. A version
row costs a vulnerability read and a licence read, and 100 is about what one
request should build. One shared number would make an operator sizing a screen
size a query at the same time.

The console's side of it differs for the same reason, and the difference is the
rule: **the catalog sends no `per_page` at all** — it *is* the list, so the
operator's number is the right one and a console asking for its own would make
the setting inert on the one screen it exists for — while the version table asks
for the 25 rows it draws above the README. Both size their pager from the
`per_page` that comes back rather than from what they asked for.

### 4.2 The new configuration parameter

```toml
[registries.readme]
remote_images = "proxy"
remote_image_hosts = ["img.shields.io", "badgen.net", "codecov.io"]
```

An entry matches the host or any subdomain of it (`shields.io` covers
`img.shields.io`, and **not** `notshields.io` — the dot is the rule). An image
from anywhere else becomes the same chip `strip` produces, so the reader still
sees that an image was there and where it pointed.

**Absent means every host**, which is what `proxy` did before the parameter
existed. A new key must not silently change what a running instance serves; an
operator who wants the narrow behaviour writes one line to ask for it. §11 O2
records the argument for the other default and why it was not taken.

### 4.3 The package page

| Control | Behaviour |
| --- | --- |
| Version filter | Substring, case-insensitive, on the version string. Says `42 of 44 shown` so a filtered list is never mistaken for a short one. |
| Pager | 25 rows a page. Absent when there is one page. A filter change returns to page 1. |
| Pre-releases | Hidden by default, one control to show them, labelled with **how many are hidden** — which is not the same as how many exist, because the selected one is always drawn. |
| Selection | A crimson left edge and raised ink, plus `aria-current="true"`. The unselected rows reserve the same 2px so marking one moves nothing. |
| `?version=` | Carries the selection when it is not the default; an unknown value falls back rather than marking nothing. The page opens on the pager page that holds it. |
| `?q=`, `?page=` | The filter and the pager, on the catalog's keys and the catalog's rules: omitted at their default, `page` 1-based because that is the number a human is looking at, and a value the controls could not have produced is clamped or dropped rather than honoured. |
| README | **View source** ⇄ **View rendered**, offered only when the endpoint returned both. |

#### The filter, the pager and the toggle are the endpoint's

All three started in the browser, over a list the endpoint sent whole. That
arrangement has two defects, and only the second one is visible from the console.

The first is cost. Assembling a package's versions enriches every row with a
vulnerability read and a licence read: 169 of each for
`@babel/plugin-transform-runtime`, to draw a table showing 25 of them. Paginating
the *answer* while still enriching every row would have moved the bytes and left
the cost, so the enrichment happens after the slice — nothing above it is allowed
to depend on those three fields, which is why the sort and the default selection
read only the version string and the source.

The second is that a client-side filter, once the answer is a page, searches what
happened to arrive rather than what this server has. "Is 4.0.2 here" would be
answered **no** about a version sitting on page three. A filter that lies in that
direction is worse than no filter.

So the parameters are the endpoint's:

| Parameter | Meaning |
| --- | --- |
| `per_page` | Rows in the answer. Absent means `[limits].versions_per_page`, which is also the ceiling; a larger ask is clamped to it and the applied value is reported back. |
| `page` | 0-based. Absent is not `0`: absent lets `version=` choose. Past the end clamps to the last page rather than answering empty. |
| `q` | Case-insensitive substring on the version string, across every page. |
| `prereleases` | `show` (the default, being what this endpoint has always answered) or `hide`. |
| `version` | The version the caller is pointing at. Survives `prereleases=hide`; does **not** survive `q`, which is a question the reader typed. Chooses the page when none is asked for. |

and the answer carries what the console says out loud — `versions_page` with
`page`, `per_page`, `total`, `unfiltered_total`, `prerelease_total` and
`hidden_prereleases`, every count over the whole list rather than the page,
because `42 of 44 shown` taken from a page is a sentence about page one wearing
the package's name.

Two facts moved to this side with them, and for the same reason: neither can be
derived from one page.

- **`default_version`** — the newest stable version this instance *holds*
  (RFC 0007 §4.2). Read off page one of a package held only at 2.1.0, "the first
  held stable row" is an upstream row we serve nothing of, which is the exact
  defect §4.2 wrote the rule to fix.
- **`selected_version`** — the version `version=` asked for, echoed back only if
  this package has it. A typo, or one yanked since the link was sent, is
  indistinguishable from "on another page" to anything holding one page.

Three of those keys describe the same list, and where they disagree the URL
wins over the page's own reflexes:

- **An explicit `?page=` outranks the jump to the selected version**, once, on
  arrival. Otherwise `?version=4.0.2&page=2` would land on the page holding
  4.0.2 and immediately contradict the address it was opened from.
- **`page=1` is not written**, because a default never is — so `?version=4.0.2`
  alone still opens on the page that holds 4.0.2, which is the behaviour a link
  naming only a version should have.
- **Reading the filter back off the URL is not typing it.** Typing returns the
  reader to page 1; hydrating must not, or every link carrying both would drop
  its page on arrival. The reset therefore hangs off the gesture — the input and
  the pre-release toggle — and not off a watcher on the state, which cannot tell
  the two apart.
- **Refresh keeps the page.** Re-reading the list under an unchanged selection is
  not a reason to move the reader, and now that the page is in the URL it is a
  position they may have arrived in from a link rather than one they scrolled to.

---

## 5. Architecture

Three boundaries move, and no new ones appear.

```text
config    registries.readme.remote_image_hosts
             ↓
core      readme::render      per-image decision: chip or keep
          readme::sanitize    `language-*` on `code`; the image rewrite
          readme::image       image_host_allowed(), the matcher both sides use
             ↓
web       explore/readme      unchanged — the console now asks it for `both`
          explore/image       re-checks the host before dialling
          explore/fetch       401 for an anonymous caller, before the download
             ↓
ui        PackageCatalog      state ⇄ query string
          PackageDetailPage   selection, filter, pager, pre-release filter
          ReadmePanel         two views, Shiki over both
          useShiki            lazy per-language loading; a DOM painter
```

The one invariant worth naming, because it is the thing an allow-list could
quietly break: **the image endpoint is addressed by index**, and the index is the
position in the list the *rendering* produced (RFC 0007-bis §5.1). So
`image_urls` takes the same host list the render took. Index 2 of a document
rendered under an allow-list is not index 2 of the same document rendered
without one, and a mismatch would serve a reader a different image from the one
the page asked about.

---

## 6. Detailed design

### 6.1 The fence language, and how much of it survives

`allowed_classes` gains a `code` entry holding whole class names —
`language-rust`, `language-js`, thirty-nine of them — rather than a
`language-*` pattern, because `allowed_classes` takes exact values and because
`language-*` sounds harmless until it is a selector some stylesheet matches. A
fence in an unlisted language renders as a plain block, which is what an
unhighlighted block is anyway.

The console's highlighter knows a superset and treats an unknown class as plain,
so the two lists may drift without either side breaking: **this one decides what
is said, that one decides what can be drawn.**

### 6.2 Per-image decisions

`strip_images` takes the host list. Empty chips every image (the `strip`
policy); non-empty chips only the images whose host is not in it, and buffers the
kept ones so they reach the sanitiser as `pulldown-cmark` produced them — alt
text is an event stream, not a string, and re-parsing it would be a second
reading of the same document.

Raw `<img>` inside markdown takes the same decision in `chip_html_images`: an
author who writes HTML must not get a different policy from one who writes
markdown.

The fetch side re-checks (`image_at`). The rendering decided what the page shows;
this decides what this server dials, and a cache sits between them — an operator
who removes a host wants the next *request* refused, not the next render-cache
miss.

### 6.3 Highlighting without a second `v-html`

`ReadmePanel.vue` is the one component allowed to render server-supplied HTML,
and `ReadmePanel.test.ts` asserts that count across the whole bundle. Shiki's
`codeToHtml` would have been safe — it escapes what it emits — and would still
have made that count two.

So the panel paints **tokens into DOM nodes**: `codeToTokens`, then a `span` per
token whose text is set through `textContent` and whose colour is set as a custom
property. Every character of package text reaches the page as text, and the only
value taken from Shiki is a colour from a theme. The count stays one, and the
argument does not have to be made a second time.

Languages load on demand from a static map (Vite needs to see the specifier to
emit a chunk). A README that fences nothing costs nothing; one that fences C++
pays for one chunk when it is opened.

### 6.4 Refusing an anonymous pull

`explore_fetch_version` returns `401` with `fetch.unauthenticated` for
`Role::Anonymous`, **after** the visibility check — a package the caller may not
see must answer `404` first, or the `401` becomes an oracle for whether a private
package exists. `fetch_offer` stops advertising the button to a session-less
reader while keeping the registry-kind reason, so a Maven registry still says
"this kind has no single artifact per version" rather than telling someone to
sign in for something signing in will not fix.

---

## 7. Security considerations

| Change | What it costs, and what pays for it |
| --- | --- |
| `class="language-…"` survives sanitisation | An enumerated set of 39 values on one element. An author's own class is still dropped; `language-rust sidebar-open` loses the second half. Tested both ways. |
| `remote_image_hosts` | Narrows what this server dials, never widens it. The permissive default preserves today's behaviour rather than quietly closing an instance's badges — an operator's silence is not consent to a change, in either direction (§11 O2). |
| README source view | Rendered as **text**, through interpolation. It is the one place the bytes must not be parsed, which is also the reason a reader opens it. |
| Shiki over package text | Tokens to `textContent`; no HTML string is constructed from package bytes anywhere in the client (§6.3). |
| Anonymous fetch refused | A reduction. The proxy path is untouched: whoever the operator's `anonymous` policy allows still downloads, and still fills the cache as a side effect. What is refused is *causing this instance to go and get something* unattributably. |
| `img-src` widened to `badge.socket.dev` | **An increase, taken deliberately and against the recommendation in this document's own drafting.** Every package page tells socket.dev which package is being read, at page load rather than on a click. The alternatives were a server-side proxy (which fails air-gapped, where the server cannot reach it either) and dropping the `<img>` for the link. The badge was kept visible. The policy says what the page *may* load; `[registries.feature_flags] socket_badge = false` still says what it *does*. |

---

## 8. Alternatives considered

1. **Keep catalog state in a store rather than the URL.** Survives Back and
   nothing else: not a refresh, not a bookmark, not a link to a colleague.
2. **`push` for the version selection.** Makes each version click undoable and
   makes Back walk out of a package one row at a time before returning to the
   search — the journey the back button exists to shorten.
3. **Highlight server-side.** Would put Shiki, a JavaScript tokeniser, in the
   Rust render path, and would cache highlighted HTML per theme. The client
   already has the highlighter for its own snippets.
4. **A `language-*` regex in the sanitiser via `attribute_filter`.** Possible —
   the filter is already there for images — and it trades an enumerated
   allow-list for a pattern, on the one attribute whose values are selectors.
5. **`remote_image_hosts` as a global rather than per registry.** The rest of the
   README configuration is per registry, and an internal registry's READMEs are
   not the public ones.
6. **Leaving the fetch button drawn and letting the `401` explain itself.** A
   control that always fails is the "disabled control with no explanation" RFC
   0007-bis §4.4 refuses, one step further along.

---

## 9. Rollout and compatibility

Nothing here requires an operator to act. `remote_image_hosts` absent is the old
behaviour; the sanitiser change adds an attribute to rendered output that older
consoles ignore; the console changes are client-side.

Two behaviours change without a switch, and both are the point of the RFC: an
anonymous caller can no longer pull, and a package page opens on a release rather
than on a pre-release. The first is a refusal an operator may want to know about
before it appears in a log; it is a `401` with a code, not a silent drop.

The rendered README's **cache key must include the renderer version**, which it
already does (`RENDERER_VERSION`) — the `language-*` change alters output for the
same input, and a stale entry would keep serving anonymous fences.

**Paginating the package-detail endpoint is the one change with a compatibility
cost, and it is worth stating plainly.** `GET /api/v1/explore/packages/…` used to
answer with every version it could assemble; it now answers with at most
`[limits].versions_per_page` of them — 100 unless an operator says otherwise. A
client that reads `versions` and assumes it holds the whole list sees the newest
100 and no error. Three things are done about it and none of them is a version
gate:

- The counts come back in the same object (`versions_page.total` against
  `versions.length`), so the shortfall is detectable in the response that causes
  it rather than by comparing against a memory of what the endpoint used to do.
- The narrowing parameters all default to the old behaviour: `prereleases`
  defaults to `show`, `q` is absent, and a caller that asks for nothing gets the
  top of the same list in the same order.
- The operator can raise the number to 1000. Not to infinity, which is what the
  old behaviour effectively was, and which the enrichment cost per row makes a
  poor default for a public console.

The only known client is this console, which was changed with it — the CLI talks
to this endpoint's `/readme` sibling and to nothing else here.
`versions_per_page = 1000` is the closest thing to the old shape for an operator
with an unknown third client and no time to fix it.

One ordering defect is fixed on the way past, and it is only a fix because the
answer is now a page: the merged list sorted **pre-releases first**, the opposite
of the comment above the comparator and of what §4.3 says the page shows. Nothing
noticed while the console received every version and arranged its own view; page
one of a beta-heavy package was entirely release candidates the moment the server
started deciding what page one is.

---

## 10. Test plan

- **Sanitiser**: a known fence language survives; an arbitrary class does not;
  `language-rust sidebar-open` keeps the first and loses the second; an unlisted
  language leaves the text and drops the name.
- **Host matcher**: empty list allows everything; an entry matches its host and
  its subdomains; `notshields.io` and `shields.io.evil.test` are refused;
  userinfo and ports are not the host; something with no host is not allowed.
- **Render**: an allowed host is proxied and the rest chipped, in markdown and in
  raw HTML; **`image_urls` returns exactly what the page was given**, which is the
  index invariant.
- **Fetch**: anonymous is `401 fetch.unauthenticated`; the storage key is still
  empty afterwards (the status code alone would pass against a handler that
  downloaded and then declined to say so); a signed-in caller the rules refuse
  still gets `403` with the rule's reason.
- **Console**: default selection prefers a held release over a held pre-release
  and over an unheld release; pre-releases hidden until asked for, and the
  selected one always drawn; filter, pager, page-1 reset, and opening on the page
  that holds a linked version; the source view is offered only when both formats
  came back, and the panel asks for `format=both`.
- **The list state round-trips**: a typed filter and a turned page are written as
  `q` and `page`; page 1 drops the key again; a link naming both opens on both
  rather than resetting to page 1; an explicit page outranks the selected
  version's own page; `page=99` clamps to the last one; and Refresh leaves the
  reader where they were.
- **The endpoint pages**: 60 versions come back 25 at a time with the totals over
  the whole list; the configured `versions_per_page` is both what an unasked-for
  page gets and what a larger ask is clamped to; `q` finds a version that is on
  the third page (the whole reason it is server-side); a filter matching nothing
  is an empty page and not an error; `page=99` clamps to the last; pre-releases
  are in the answer until `prereleases=hide`, which reports what it removed;
  `version=` survives that filter, chooses its own page, is outranked by an
  explicit one, and is not echoed back when the package does not have it; a
  pre-release-only package still answers with a row; and `default_version` is
  the newest **held** release rather than the newest that exists.
- **The catalog pages on the operator's number**: an unconfigured server answers
  20 and says so; a configured one answers that many; asking for more is clamped
  to it and asking for less is honoured; `per_page=0` is still clamped to one row
  (it collapses the listing query's cache key onto the count query's); and
  setting one page-size key does not move the other. The console sends no
  `per_page`, sizes its pager from the answer, and falls back to 20 when an
  answer omits it.
- **The console asks rather than filters**: the query carries `per_page`,
  `prereleases` and the typed `q`; a burst of typing is one request, not one per
  letter; a superseded answer is dropped; the page is taken back from the
  response rather than from what was asked for; and the page marks the version
  the server names, including one no client-side rule would have picked.
- **Measured in a browser, not only in jsdom**: 44 versions → 25 a page, `42 of
  44 shown`, `chalk`'s seven fenced blocks highlighted, the source view at 13 884
  characters with 897 token spans, and a search restored — URL, box, sort,
  registry and scroll — after opening a result and pressing Back.

---

## 11. Decisions and open questions

| # | Question | Answer |
| --- | --- | --- |
| O1 | Should the filter and the page live in the URL, like the version? | **Yes — answered against the first draft.** It said *not yet*: a version is a destination someone sends, "page 3 of a filter" is a position in a session. The distinction did not survive contact with a 169-version package, where the position *is* the destination — "the four 4.0.x builds", "the page the 2019 releases are on" — and it was lost to a reload, to the page's own Refresh, and to being pasted to a colleague. It is the same mechanism again, on the catalog's keys: `?q=` and `?page=`. §4.3 records the three precedence rules that fall out of having a second author for the same state. |
| O2 | Should `remote_image_hosts` default to *deny*? | **No, and this is the uncomfortable one.** Deny-by-default is the better posture and would silently blank the badges of every instance already running `proxy`. The compatible reading ships; a `[registries.readme]` block with `proxy` and no list is a candidate for a config *warning* rather than a changed default. |
| O6 | Should the console keep its own 25-row page now that the server pages? | **Yes.** The two numbers answer different questions: `[limits].versions_per_page` is how much of a list this server will build for one request, the console's 25 is how many rows fit above the README without turning the page into a table. The console asks for what it draws and reads back what was applied, so an operator who sets the ceiling *below* 25 gets a console that follows rather than one that silently shows a page of the wrong length. |
| O3 | Should the pre-release toggle's state be shared? | Still no, and no longer for O1's reason. A link naming a pre-release already reveals it, which is the case that matters; the toggle changes what the list *is* rather than where in it you are. If it goes in, it is a third key on the mechanism O1 built. |
| O4 | Should the README source view be highlighted at all? | Yes — it is a code block, and Shiki paints it without changing a character. It costs the `markdown` grammar chunk, loaded only when someone opens the view. |
| O5 | Does anything else in the console draw a third-party image? | Unmeasured beyond the socket.dev badge. Worth one pass before the CSP is treated as settled. |

---

## 12. Implementation phases

| # | Phase | What it changed |
| --- | --- | --- |
| 1 | Catalog state in the URL, `router.back()`, `scrollBehavior` | `PackageCatalog.vue`, `PackageDetailPage.vue`, `router/index.ts` |
| 2 | Upstream rows open their package page | `PackageCatalog.vue` — and the test that pinned the old refusal |
| 3 | Selection: ink not fill, `aria-current`, held-release default, `?version=` | `PackageDetailPage.vue` |
| 4 | Full-bleed page, display-face `h1`, README measure | `PackageDetailPage.vue`, `ReadmePanel.vue` |
| 5 | Anonymous pull refused; the offer withdrawn | `explore/fetch.rs`, `explore/detail.rs` |
| 6 | Pre-releases opt-in; version filter and pager | `PackageDetailPage.vue` |
| 7 | Fence language through the sanitiser; Shiki over both views; source toggle | `readme/sanitize.rs`, `useShiki.ts`, `ReadmePanel.vue` |
| 8 | `remote_image_hosts` end to end | `config/schema/registry.rs`, `hot_config.rs`, `readme/{render,image,mod}.rs`, `explore/image.rs`, `server/hot_config.rs` |
| 9 | The filter and the page in the URL (§11 O1, reversed) | `PackageDetailPage.vue` — one `syncQuery` for all three keys, the page-1 reset moved onto the gesture, and the jump-to-selection taught to yield to `?page=` and to sit still through a Refresh |
| 10 | The version list is paged, filtered and sorted **server-side** | `config/schema/mod.rs` (`[limits].versions_per_page`), `core/services/hot_config.rs` (the default constant both crates read), `server/hot_config.rs`, `explore/detail.rs` (the five parameters, `versions_page`, `default_version`, `selected_version`, enrichment moved after the slice, pre-release sort order fixed), `PackageDetailPage.vue` (the controls send; debounce, sequence token, silent refetch) |
| 11 | The catalog's page size is the operator's | `config/schema/mod.rs` (`[limits].packages_per_page`, one validation loop for both keys), `core/services/hot_config.rs`, `server/hot_config.rs`, `explore/list.rs` (the `serde` default 20 replaced by the configured value as default *and* ceiling), `PackageCatalog.vue` (`const perPage` → the server's, read back and cached with the rows) |

---

## See also

- [RFC 0007 — The README, per version](/rfc/0007-package-readmes) — where `source_text` came from, and the boundary §6.3 keeps
- [RFC 0007-bis — The three 0007 deferred](/rfc/0007-bis-images-search-and-fetch) — the image proxy this RFC narrows, and the index invariant it must not break
- [RFC 0012 — Signed URLs for the credential-less request](/rfc/0012-signed-urls-for-terraform) — the other half of "who is allowed to make this server fetch something"
