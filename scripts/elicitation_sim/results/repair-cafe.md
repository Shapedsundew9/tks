---
format: tks-transcript/v1
title: "Community Repair Cafe"
session_id: "repair-cafe"
cycle: 1
date: "2026-10-24"
language: "en"
source: "simulation"
participants:
  - { name: "Ana", role: "stakeholder" }
  - { name: "Facilitator", role: "facilitator" }
---

**[t1] Ana (stakeholder):** We run a small community repair cafe where volunteers fix people's broken household items on Saturdays, but we keep losing track of what came in and who is working on it.

**[t2] Facilitator (facilitator):** [A3: Core Operational Invariant] If a volunteer starts working on an item but leaves it incomplete before the next Saturday, does the domain rule require the item to be re-tracked from scratch or carried over to the next session?

**[t3] Ana (stakeholder):** The item is carried over to the next session; every item keeps its record with an assigned fixer, owner, and status until resolved.

**[t4] Facilitator (facilitator):** [Exception & Conflict Resolution] If a volunteer leaves an item incomplete and another volunteer takes over, does the domain rule require the new volunteer to assume full responsibility for the item's progress or must they start tracking it as a new entry?

**[t5] Ana (stakeholder):** They do not start a new entry. The new volunteer becomes the assigned fixer on the existing item, assuming responsibility for its progress and any required safety sign-offs.

**[t6] Facilitator (facilitator):** [Temporal Cadence & Lifecycle] If an item remains unresolved past the next Saturday, does the domain rule require it to be automatically reassigned to a new volunteer or marked as "unresolved" for the owner to claim and resolve independently?
