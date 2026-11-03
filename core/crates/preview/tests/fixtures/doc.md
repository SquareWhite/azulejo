---
title: Front matter is not rendered
---

# Plan: preview tiles

Some *emphasis*, **strong**, ~~struck~~ and `inline code`. A bare link: https://example.com/docs.

## Tasks

- [x] Render markdown
- [ ] Watch files

| Step | Owner | Done |
| ---- | :---: | ---: |
| 1.9a | core  | yes  |
| 1.9b | shell | no   |

A footnote reference[^why].

[^why]: Because the shell needs HTML.

## Links and images

![diagram](<img/flow chart.png> "Flow")
[the PRD](../docs/PRD.md#goals), [section](#tasks), [mail](mailto:me@example.com)
![remote](https://example.com/logo.png)

<script>alert("raw html is dropped")</script>
<b>also dropped</b>

[bad](javascript:alert(1))

```rust
fn main() {
    println!("hello <world> & \"friends\"");
}
```

```unknown-lang
plain & escaped <text>
```

> A quote with `code`.
