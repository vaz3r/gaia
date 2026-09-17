# Classifier Model Artifact History

This document records retired classifier model artifacts for audit and recovery purposes.
All listed artifacts are recoverable from Git history at the commits noted below.

| Version | Filename | SHA-256 | Size | Activated At | Commit |
|---------|----------|---------|------|-------------|--------|
| v2 | torrent_classifier_v2.joblib | 989e96c4c37c4f1c81a66fc3f394dbf6b0b4638e83945ec47126816ae3beeb0a | 17.06 MB | 2026-09-06T10:11:10Z | 17c5559 |
| v3 | torrent_classifier_v3_20260908_150633.joblib | e86dd70befb44df24b8b680008b3bd836806634e85175a83139c879c15a465c4 | 16.21 MB | 2026-09-08T11:06:35Z | c0e644d |
| v4 | torrent_classifier_v4_20260908_152230.joblib | 1d4a54113ed32aaf27ea4b7b4ccab34c5dd4839abce516b7b1092ade6ea59a97 | 15.76 MB | 2026-09-08T17:29:33Z | 847d34e |
| v5 | torrent_classifier_v5_20260911_144036.joblib | b9917194430b65f47ff2bfeca7a635566387f4f3b1eb3e02cddc0e641cf2c6d5 | 19.77 MB | 2026-09-11T10:40:38Z | 9b43d30 |
| v6 | torrent_classifier_v6_20260911_155955.joblib | bc5363e0086f9580f802b8dc2d019e95dd8d7876a21d9647d8b7b1fdb1afb5ad | 19.89 MB | 2026-09-11T11:59:57Z | 9b43d30 |

## Recovery

To recover any retired model:

```bash
# Example: recover v2
git show 17c5559:apps/classifier/models/torrent_classifier_v2.joblib > torrent_classifier_v2.joblib
```

## Notes

- v7 was activated then superseded by v8 on the same day (2026-09-12); its model file was already absent from the repository before this cleanup.
- The active model is v8 (`torrent_classifier_v8_20260912_212717.joblib`).
- Historical model metrics are preserved in `apps/classifier/models/active_model.json` under the `history` array.
