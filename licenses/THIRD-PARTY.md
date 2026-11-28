# Third-party runtime dependencies

`billet` (the `blt` binary and the library) depends on serde_json for editing
agent configuration during `blt init`. The resolved runtime graph (`cargo tree
--locked --edges normal`) and each crate's license:

| Crate | Version | License | Text |
|---|---|---|---|
| serde_json | =1.0.154 | MIT OR Apache-2.0 | `MIT`, `APACHE-2.0` |
| itoa | 1.0.19 | MIT OR Apache-2.0 | `MIT`, `APACHE-2.0` |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | `MIT`, `APACHE-2.0` |
| zmij | 1.0.23 | MIT | `MIT` |
| memchr | 2.8.3 | Unlicense OR MIT | `UNLICENSE`, `MIT-memchr` |

The `MIT` file is the identical text shipped by serde_json, itoa, serde_core
and zmij. `MIT-memchr` is memchr's own MIT text (copyright Andrew Gallant);
memchr also offers the Unlicense in `UNLICENSE`.
