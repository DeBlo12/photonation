# Lensfun lens database

The files in `db/` are the lens database of the
[Lensfun project](https://lensfun.github.io/), copied unchanged from
<https://github.com/lensfun/lensfun/tree/master/data/db> at commit
`bc5615d6278cb5fb75dd53e74015367453d5591d` (28 October 2026).

Copyright the Lensfun project and its contributors. The database is licensed
under the Creative Commons Attribution-ShareAlike 3.0 Unported licence
(CC BY-SA 3.0); the full text is in `COPYING.CC_BY-SA_3.0` and at
<https://creativecommons.org/licenses/by-sa/3.0/>.

Photonation embeds these files in its binaries as they are and reads them at
run time (`crates/lens`). They are a separate work aggregated with
Photonation, not part of its GPL-3.0 code, and keep their own licence. To
update them, copy the upstream `data/db` directory over `db/` without editing
any file, and update the commit above.
