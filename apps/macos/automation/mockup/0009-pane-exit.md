ADR 0009 · Accepted

# Close the tile when its shell exits

A deliberate exit (`ctrl+d`, `exit`, a finished command) closes the tile. Neighbours take the space and focus moves on.

```toml
[terminal]
on_exit = "close"  # or "keep"
```

"Session ended" shows only when herdr loses a pane it never saw exit.
