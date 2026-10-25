# Security policy

Photonation parses untrusted files (camera raws), so decoder and pipeline bugs
can have security impact.

## Supported versions

Only the latest commit on `main` receives fixes until the first release.

## Reporting a vulnerability

Please **do not** open a public issue. Report privately through GitHub:
**Security → Report a vulnerability** on
<https://github.com/DeBlo12/photonation/security/advisories/new>.

Include the affected commit, steps to reproduce and, if possible, a sample file
(only one you may share). You will get an acknowledgement within a week; fixes
are coordinated with you before disclosure.

Issues in third-party crates (for example the raw decoder, rawler) are best
reported upstream as well.
