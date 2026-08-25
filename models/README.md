# Frozen policy models

These directories are immutable inference inputs. A directory name is only a
label; `manifest.json:model_id` and `checkpoint_sha256` are authoritative.
Never replace `checkpoint.pt` in place. A model update gets a new directory and
a full graph reattestation before an atomic snapshot publication.

Current v0 model:

```text
q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63
```

It is the `raster-axial-12` scientist copied from one read of the recorded Q254
state on 2026-08-24. The manifest records source-state provenance and explicitly
sets `training_performed` to false.
