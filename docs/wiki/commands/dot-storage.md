# .storage / .storages — List Storage Names

Machine-interface command used by bconsole/oconsole for tab-completion. Returns one storage resource name per line from the Director configuration.

## Syntax

```
.storage [enabled|disabled]
.storages [enabled|disabled]
```

## Arguments

| Argument | Description |
|----------|-------------|
| *(none)* | List all storages |
| `enabled` | List only enabled storages |
| `disabled` | List only disabled storages |

## Examples

### List all storages
```
* .storage
File
TapeLibrary
```

### List only enabled storages
```
* .storages enabled
File
```

### List only disabled storages
```
* .storage disabled
TapeLibrary
```

## Notes

- Both `.storage` and `.storages` are accepted (same handler).
- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Used by console programs to populate the `storage=` argument completion list.

## Related Commands

- [status](status.md) -- Show storage daemon status
- [.defaults](dot-defaults.md) -- Show default values for a storage
- [.mediatypes](dot-mediatypes.md) -- List media type names
