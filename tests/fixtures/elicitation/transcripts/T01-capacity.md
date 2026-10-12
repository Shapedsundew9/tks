---
format: tks-transcript/v1
title: "Engineering Team Capacity Planning"
session_id: "T01"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
  - { name: "Marcus", role: "facilitator" }
---

**[t1] Marcus (facilitator):** Thanks for joining, Priya. Could you walk me through the primary challenges your team leads face when planning capacity across engineering sprints?

**[t2] Priya (stakeholder):** Honestly, I spend half my Monday chasing people on Slack just to figure out who has bandwidth. We really need team leads to be able to assess sprint progress without interrupting developers constantly.

**[t3] Marcus (facilitator):** When you say assess progress without interrupting developers, what does that look like practically?

**[t4] Priya (stakeholder):** In my head, it is like an Excel sheet with pivot tables where you can slice team allocations by project and see instant totals.

**[t5] Marcus (facilitator):** Understood. Beyond viewing allocations, what happens when an unexpected incident occurs?

**[t6] Priya (stakeholder):** That is our biggest friction point right now. When production incidents happen, we have to maintain dependable on-call coverage without burning out the three people who know the deployment pipeline.

**[t7] Marcus (facilitator):** Who currently handles the schedule redistribution when on-call duties conflict with planned deliverables?

**[t8] Priya (stakeholder):** Right now, nobody does cleanly. That bottleneck causes delivery slippage every quarter. And maybe we could forecast our quarterly hiring needs from velocity deficits, but we have not agreed on that yet.

**[t9] Marcus (facilitator):** How do you envision the system managing individual developer workloads?

**[t10] Marcus (facilitator):** What if we had an automated background bot that continuously inspects active developer commits and Slack statuses to compute real-time capacity?

**[t11] Priya (stakeholder):** Well, I suppose so, but that sounds quite risky to me.

**[t12] Marcus (facilitator):** Can you elaborate on the risk you see there?

**[t13] Marcus (facilitator):** Is developer privacy a concern when gathering workload telemetry?

**[t14] Priya (stakeholder):** Exactly. To be clear on boundaries, we definitely do not want to track individual keystrokes or active hours, because surveillance destroys engineering trust.

**[t15] Marcus (facilitator):** That is an important operational boundary. What about long-term predictability?

**[t16] Priya (stakeholder):** Our engineering leads need to detect team overcommitments before sprint kickoff so commitments remain realistic.

**[t17] Marcus (facilitator):** What feeling or outcome does success create for your engineering organization?

**[t18] Priya (stakeholder):** The whole engineering organization should feel a sense of calm and predictability rather than constant panic.

**[t19] Marcus (facilitator):** Are there other dependencies between teams that complicate these commitments?

**[t20] Priya (stakeholder):** Yes, platform dependencies routinely stall product squads, but we need to tackle baseline capacity visibility first.
