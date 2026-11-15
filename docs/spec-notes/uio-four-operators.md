# Four UIO operators needed by held map recipients

[documented] UIO Extra XML Operators (https://geckwiki.com/index.php?title=UIO_Extra_XML_Operators) defines sine and cosine of a degree argument, natural logarithm of the argument and the accumulated value raised to the exponent argument. These operator contracts are independent of native script math functions.

[observed: private installed XML metadata only] The inspected held-map/HUD/quest subtrees use Sin/Cos/Log/Pow and no other missing extra operators. Main held-map XML has 3 sine, 3 cosine, 2 log, 2 pow occurrences; its separate MCM pages reuse the same four. No installed XML/script content is transcribed into repository source or fixtures.

[hypothesis] Add only these four parser/evaluator cases. Existing operators, counted source paths, UIO processing and font behavior remain unchanged. Undefined/nonfinite added results become numeric zero; this is an explicit engine safety policy, not native edge-case parity. Degree source changes recompute through the existing dependency machinery. Five authored parser/tree controls are prepared; all are UNCOMPILED/UNRUN in this source-only packet. Prior 103 UI proof predates these four operators.

[hypothesis] The authored `ui-four-extra-operators` TOOL scenario invokes the existing `fnvtool ui tree` consumer with a repository-owned XML fixture and no game assets. It requires positive/negative nonzero trig results, a log ratio, accumulated power, an ordinary arithmetic neighbor and a nonzero sentinel. It is authored but UNRUN; the referenced-angle mutation remains covered only by the authored CPU control, also UNRUN. All incoming smoke scenarios and expectations are retained.
