# A1 Provider/store matrix

| Direction | Egress | Offline | model/dimension | deterministic cache | license/privacy | result |
|---|---|---|---|---|---|---|
| Existing lexical Tantivy | none | yes | n/a | yes | existing Apache-2.0 dependency | retained fallback |
| Local embedding runtime | none after model provision | conditional | UNVERIFIED | must be content-hash keyed | model license UNVERIFIED | `UNVERIFIED`, no A2 dependency |
| External embedding API | required | no | provider-specific | provider-dependent | disallowed before owner decision | rejected for A1 |

No network or credential was used. A1 therefore makes no claim about model quality, cost, hardware or external license. Vector capability must report typed `Unavailable` without breaking exact/FTS until a later owner-approved provider gate.
