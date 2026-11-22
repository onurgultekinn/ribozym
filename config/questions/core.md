# Core question set (v2.9 / v2.11)
# ONE universal set, static file — the agent NEVER invents new questions.
# Its only job (A1 cheap mode) is to ADAPT each generic question to the current topic
# at ask-time ("what data do you store?" -> "where should calendar events and user
# records be stored?"). Answers are written into spec_form.md by CODE; no analysis
# happens in this stage. 2-4 questions per round, max 3 rounds (§5.4 Stage 1).

# Round 1 — purpose & users
- q1: What is the goal of the project, in one sentence?
- q2: Who are the target users, and where are they (region/platform)?
- q3: What is the single most important thing this project must do well?

# Round 2 — scope & constraints
- q4: Which features are absolute must-haves for the first version?
- q5: Which features are explicitly OUT of scope for now?
- q6: What constraints bind the project (tech stack, existing systems, deadline, regulations)?

# Round 3 — money, delivery, success
- q7: What is the development budget for this run, and what runtime/hosting budget exists?
- q8: What is the desired delivery shape (CLI tool, web service, library, container...)?
- q9: How will you measure success — what observable result means "this works"?

# Notes for adaptation (A1 cheap mode):
# - Keep every question universal; only specialize wording to the user's topic.
# - Never merge two questions; never drop a topic, even if the user preempted it
#   (record the pre-answered topic and move on).
# - Output is ONLY the adapted question sentences (very short = very cheap, §5.4 Stage 1).
