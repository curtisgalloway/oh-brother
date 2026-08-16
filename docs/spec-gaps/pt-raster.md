<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Spec gaps — docs/pt-raster-h500-spec.md

Questions the spec could not answer, per its usage notice. Resolve
from the Brother PDF or on hardware — never from third-party driver
source. Mark resolved entries with the answer and evidence.

- [resolved 2026-08-15] §6.1/§6.2 — does the H500 family apply the
  ESC i d amount to one end or both ends of the print area? Hardware
  answer (PT-H500, 12 mm TZe): end margin only, same semantics as the
  Cube. Default 174 dots produced visually equal ~24.5 mm margins;
  --save-tape (14 dots) left ~2 mm trailing blank with the mechanical
  ~24.5 mm lead unchanged; both labels cut/ejected normally.
- [resolved 2026-08-15] §7 — is the full per-page preamble (ESC i a /
  i z / i M / i K) required for ESC i d to be honored on this family?
  No: the margin is honored on the minimal path, sent as ESC i d
  before `4D 02` + `1B 69 52 01` with no other control codes
  (PT-H500 hardware test above).
- [open] 2026-08-15 §6.1 — the exact meaning of the databook's
  "minimum margin setting with no precut" row (24.3 mm / 172 dots):
  garbled in the text extraction; re-read the PDF (Drive file
  1UAp_Efs6NSkBN737CMb4R8nodjWTqq1A) §2.3.3 directly.
- [open] 2026-08-15 §5.3 — cooling-notification numeric codes and the
  4-byte hardware-settings status field: illegible in the extraction;
  re-read the PDF §4 ESC i S if ever needed.
