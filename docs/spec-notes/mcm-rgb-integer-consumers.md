# MCM integer RGB consumers

## Documented operators and original demand

[documented: GECK wiki, [UIO Extra XML Operators](https://geckwiki.com/index.php?title=UIO_Extra_XML_Operators)]
UIO adds accumulator bitwise AND (`land`), left shift (`shl`) and arithmetic right
shift (`shr`). These are distinct from logical `and`. The documented arithmetic
right shift is not an instruction to choose a particular integer width.

[observed: original MCM Extender 1.6.3 OptionRect and HSVColorPicker XML]
The option preview extracts red/green/blue from a packed colour using shifts
16/8/0 and mask 255. The picker rebuilds a packed colour from channel values
shifted by 16/8/0, adding their disjoint contributions. This is a consumer-demand
observation from original data, not an original-executable numeric-parity capture.
No original XML is included here.

## Bounded acceptance contract

[design] The first supported consumer domain is finite integer channels 0 through
255, packed integers 0 through 16777215, mask 255 and shift counts 0, 8 and 16.
These values remain exactly representable even in a binary32 trait. Within this
domain the required result is independent of choosing 32-bit versus wider signed
integer arithmetic: unpacked channels equal their independent input values, and
packing equals `red * 65536 + green * 256 + blue`. No implementation, clamp,
conversion, validation or rejection policy is selected by these fixtures.

[hypothesis] General integer width, sign conversion, fractional values, nonfinite
values, overflow, negative/out-of-range shift counts and negative packed colour
sentinels remain unknown. They need explicit permitted captures and a reviewed
mapped policy before broad operator-parity claims. The existing MCM system-colour
path and its negative/default sentinels must retain their existing treatment.

## Fixtures and actual boundary

Three authored `fnv-ui` libtests load independent menus through the ordinary XML
reader, expression tree, trait setters and update scheduler. They use an option
preview's actual image red/green/blue traits and a picker output consumed by that
preview. Distinct channel values, all-zero/all-255 endpoints, live channel edits,
stable updates, a fresh tree receiving the selected value and an ordinary
arithmetic neighbor provide controls. Expected packed values use integer
place-value arithmetic rather than the implementation under test.

The first intended old-consumer failure is a wrong swatch channel or packed
output, before a later parse-warning assertion. Ignored-operator warnings alone
do not prove pixel output, physical UI interaction or full mod acceptance.
All three cases are unexecuted. The shared parser/evaluator is unchanged while
operator ownership is reconciled. No native renderer, installed MCM options,
original HSV rounding, input/persistence or UIO-wide compatibility is certified.
