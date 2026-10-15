# .grpc — Toggle gRPC Streaming Mode

Switches the session into gRPC streaming mode for use by gRPC-based frontends. This is an Onesimus Director extension not present in C++ Bareos.

## Syntax

```
.grpc [on|off] [format=<proto|json|text>] [compact=<yes|no>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `on` | Enable gRPC streaming mode |
| `off` | Disable gRPC streaming mode |
| `format=proto` | Use Protocol Buffers encoding |
| `format=json` | Use JSON encoding (default) |
| `format=text` | Use text encoding |
| `compact=yes` | Use compact output |
| `compact=no` | Use pretty-printed output |

## Examples

### Enable gRPC mode
```
* .grpc on
gRPC mode: on, format=json
```

### Enable with protobuf format
```
* .grpc on format=proto
gRPC mode: on, format=proto
```

### Disable gRPC mode
```
* .grpc off
gRPC mode: off
```

## Notes

- Returns a confirmation message with the current gRPC state.
- Output is terminated by the EOD signal.
- When enabled, subsequent command responses are streamed as gRPC messages instead of plain text.
- This is specific to the Onesimus Director and is not compatible with C++ Bareos.

## Related Commands

- [.api](dot-api.md) -- Set API output mode (human/machine/JSON)
