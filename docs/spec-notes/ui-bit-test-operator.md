# Bit tests in menu XML expressions

[documented] The GECK Wiki [UIO Extra XML Operators](https://geckwiki.com/index.php?title=UIO_Extra_XML_Operators)
table (revision 39959, retrieved 2026-11-16) describes `bt` as testing bit `p`
of the current expression value, producing numeric one when set and zero when
clear. The argument selects a bit; it is not a mask. This note concerns XML
expressions, not a script command with a similar name.

[documented: NVForever source] `fnv-ui` loads operators through `OpKind::from_tag`
and resolves literal, nested and referenced arguments in `Ui::eval_reading`.
Traits use `f64` numbers. Unknown tags produce a warning and are ignored.

[hypothesis: engine policy] Admit `bt` case insensitively through the existing
parser and evaluate it after its argument has been resolved. Support nonnegative
whole-number accumulators through 2^53 inclusive, where consecutive integers are
exactly represented by the trait number type, and nonnegative whole-number bit
indexes. Positions above 53 are clear in this domain. Other numeric inputs return
zero. Existing `Value::num` coercion still applies before these checks. No native
integer width, signed conversion, truncation or wrapped shift is inferred.

[hypothesis: validation scope] Authored tests use the real menu loader, setters
and reactive update consumer. They cover zero/set/clear bits, argument versus
mask interpretation, composition, nested and live arguments, changes to either
operand, deferred evaluation, direct overrides and cold reloading. Separate
controls retain ordinary boolean/arithmetic behavior and unknown-tag warnings.
Boundary controls cover the exact-integer limit and explicitly selected invalid
input policy. These controls have not yet executed on this source.

Native edge behavior remains unobserved: negative or fractional operands,
nonfinite values, indexes outside the native width and numeric precision/conversion
need original-game UI probes before any parity claim. No native capture,
installed-mod acceptance or plugin-version compatibility is established here.
