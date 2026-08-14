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
Three of the six below are implemented and three are not. The banner is
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

They read in order: each one argues with the state the previous one left.

The RFC template itself is not published — it is a form you copy, not a
document you read. It lives in the repository at `docs/internal/0000-rfc-template.md`.
