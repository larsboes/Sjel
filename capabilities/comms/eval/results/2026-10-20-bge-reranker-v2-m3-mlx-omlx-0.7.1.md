# bge-reranker-v2-m3-mlx on oMLX 0.7.1 - 2026-10-20

- Model: `bge-reranker-v2-m3-mlx`
- Source: `soichisumi/bge-reranker-v2-m3-mlx`
- Revision: `b4577f49e18adb53ed9e557192094f69f3dc2c1c`
- License: Apache-2.0
- Runtime: oMLX 0.7.1, loopback `/v1/rerank`
- Corpus SHA-256: `74a54a928a62f9b676b5bcd36d596a71613e2ef0aed23583cd307bb0541cf701`
- Weight SHA-256: `80be6e38dfd2156d865a5068cdd78774f29b4b91ce100acc9f331c382e2b18b4`

The same pinned tree as [the 2026-08-04 baseline](2026-08-04-bge-reranker-v2-m3-mlx.md), downloaded
again after oMLX returned to this host. Both Git LFS objects matched their published hashes before
oMLX loaded them.

## Result

```text
local-ai-resources-de: top=mlx-memory-en score=0.266 judgement=3 nDCG=1.000
architecture-boundaries-en: top=contracts-de score=0.024 judgement=3 nDCG=1.000
migration-research-de: top=dsr-migration-en score=1.000 judgement=3 nDCG=1.000
rail-reliability-en: top=punctuality-data-de score=0.150 judgement=3 nDCG=0.993
scholarship-eligibility-de: top=funding-criteria-en score=0.859 judgement=3 nDCG=1.000
career-visibility-en: top=cfp-open-source-de score=0.012 judgement=3 nDCG=0.972
top1-relevant=1.000 pairwise=0.941 mean-nDCG=0.994 result=PASS
```

The corpus and judgements are unchanged, and the run passes every threshold again. Against the
0.5.0 run, pairwise accuracy rises from 0.912 to 0.941 and mean nDCG from 0.993 to 0.994. The top
candidate for each query is the same. The difference comes from the runtime, not the weights.
