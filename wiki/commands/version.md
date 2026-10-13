# version — Show Director Version

Display the Director daemon version, build info, and operating system.

## Syntax

```
version
```

## Example

```
* version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (Debian GNU/Linux 13 (trixie))
```

## Notes

The version string format is `bareos-dir Version: <version> (<os>)` for protocol compatibility with C++ bconsole and Bareos WebUI. The actual daemon binary is `onesimus-dird`.
