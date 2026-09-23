<!-- work-compatibility-revision: 1 -->
# Load routed references


1. Do not enumerate or eagerly load every file below a hierarchy node's `references/` directory.
2. When a loaded `instructions.md` explicitly routes to a relative reference and its trigger applies:
   1. Resolve it only relative to the directory containing that `instructions.md`.
   2. Require it to be a regular file inside that same hierarchy directory after platform normalization and link resolution.
   3. Load it immediately after the declaring instruction using its declared globally unique reference name.
3. Decode routed references using the same strict UTF-8 and optional BOM behavior as instruction files.
4. Apply only references whose trigger is established by the current request or confirmed task state. Record both loaded references and references whose triggers did not apply.
5. Re-evaluate reference triggers when new evidence or a confirmed decision changes applicability. Add newly applicable references, remove references that no longer apply, rebuild actual source order and instruction fingerprints, list affected decisions, and reconfirm each affected decision before formal TASK approval.
