# Design history

For someone asking why it is like this.

Every substantial change to BatleHub — a user-facing surface, a cross-crate
refactor, a security-relevant default, anything expensive to undo — is argued
out in an RFC before it is built. These are those documents, published
unedited.

They are candid about defects that were live in shipped versions, and that is
deliberate. For a self-hosted infrastructure product, the useful thing to know
is not that a project has never been wrong; it is what it does when it finds
out.

**Every page here opens with a status banner, and the status is not uniform.**
Eight of the fifteen below are implemented and seven are not. The banner is
generated from each document's own `Status` field rather than written on the
page, so it cannot drift from the table it is quoting — an RFC that describes a
proposal, published under a label saying it shipped, would be a claim about the
product that is not true.

| RFC | What it settles |
| --- | --- |
| [0001 — Subdomain routing](/rfc/0001-subdomain-routing) | Reaching a registry by host name instead of by path |
| [0002 — Vulnerability flags and exposure](/rfc/0002-vulnerability-flags-and-exposure) | What BatleHub knows about a package's CVEs, and who it tells |
| [0003 — UI rework](/rfc/0003-ui-rework) | The design system the console and this site both wear |
| [0004 — Admin composition and API surface](/rfc/0004-admin-composition-and-api-surface) | The API the console was missing |
| [0004-bis — What RFC 0004 left](/rfc/0004-bis-what-rfc-0004-left) | The parts 0004 did not finish, and why they were not visible |
| [0005 — One documentation tree](/rfc/0005-docs-site-design-system) | Merging the two documentation trees, and putting the design system on the result |
| [0005-bis — Two readers, one home each](/rfc/0005-bis-audience-split-and-one-home) | Splitting the guide by audience, giving every instruction one home, cutting each page down to one subject, and turning the showcase back into an introduction |
| [0006 — A block every ecosystem can see](/rfc/0006-blocked-versions-hidden-everywhere) | Hiding blocked versions from every registry's listings, not just npm's, and stating which protocols cannot be filtered |
| [0007 — The README, per version](/rfc/0007-package-readmes) | Storing each version's own README, rendering it safely on the server, and making the package page answer — versions and documentation — for packages this instance holds nothing of |
| [0007-bis — The three 0007 deferred](/rfc/0007-bis-images-search-and-fetch) | Rendering a README's images without telling their host who is reading, searching what a package says rather than what it is called, and asking for a version the page has told you exists |
| [0008 — mise in an air-gapped estate](/rfc/0008-mise-in-an-air-gapped-estate) | Making `mise install` work with no route off the site: `mise.lock` as the bill of materials, a server that will not dial out, and verification moved to the connected side |
| [0009 — Every endpoint the client actually calls](/rfc/0009-protocol-coverage) | Serving the paths each package manager really requests, and two mechanisms so the next invented endpoint fails the build |
| [0010 — The toolchain layer](/rfc/0010-toolchain-managers) | Proxying the JDK and the Node runtime themselves, not only what they build: SDKMAN and the `nodejs.org/dist` tree as registry kinds, and making a blocked toolchain a refusal rather than a claim |
| [0012 — Signed URLs for the credential-less request](/rfc/0012-signed-urls-for-terraform) | Letting a client that sends no credential — Terraform's provider archive — download from a registry that is closed to everyone else |
| [0013 — What the console owes a reader](/rfc/0013-console-answers-for-a-package) | Eleven things the package pages knew and could not act on — a search that survives a click, a version that is a link, a README you can read as source, the hosts an image may come from, and two lists that page on the operator's numbers rather than on a literal |

0007 and 0008 were deferred behind 0009, and the reason was worth stating plainly:
0009 found six protocol defects that had all shipped green, and the common cause
was tests written from our implementation rather than from what the client sends.
Building further on that foundation before fixing it would put more surface on a
floor we know does not hold. 0008's air-gapped case also depended directly on
0009's checksum-database and upstream-caching work.

0007-bis picks up the three open questions 0007 recommended a decision on and did
not take — the image proxy, prose search, and a way to fetch a version the page
has just told you exists. All three are the same shape: the page knows something
it cannot act on. It has since landed, and it is the one in this set whose open
questions were settled by **measuring** rather than by argument: two of the five
were resolved against the recommendation it was drafted with, because 67 % of the
images in real READMEs turned out to be SVG and because `simple` full-text search
cannot find a README that says `retrying` when a reader types `retry`.

0009 has since landed, and with it that dependency: the Go checksum database is
proxied *and cached*, so a second build resolves with no route off the site, and
Terraform's checksum files no longer send an otherwise air-gapped provider
install to the internet at its last step. 0007 has since landed on that
foundation — reusing 0009's three rungs for its discovery read rather than
inventing a second cache policy — and 0008 is waiting on scheduling rather than
on 0009; see its own §2.7 for what that leaves.

0010 is the first to argue with what the *other* RFCs assumed rather than with
what they built. 0006 made a block visible in every ecosystem's listings, and
0009 made every endpoint the one the client really calls — for the dependency
layer. The layer underneath it, the JDK and the Node runtime a project builds
*on*, is a row in the console that nothing enforces. Its two halves fail
differently, which is why they are argued together: SDKMAN has never reached a
BatleHub instance at all, while `nodejs.org/dist` has been mirrored as a
`generic` registry all along — caching every byte and enforcing nothing, because
a path-addressed mirror has no version to block. The cache working is what made
that one invisible. It is also the rest of the estate 0008 described: `mise`
covers some toolchains, SDKMAN and nvm cover the others, and an air gap has to
hold for all of them.

They read in order: each one argues with the state the previous one left.

The RFC template itself is not published — it is a form you copy, not a
document you read. It lives in the repository at `docs/internal/0000-rfc-template.md`.
