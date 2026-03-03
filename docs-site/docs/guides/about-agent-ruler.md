---
title: About Agent Ruler
---

# About Agent Ruler

I started the Ruler because I wanted an agent that can actually do useful things on my computer, without me supervising it all day like a babysitter.

We have work to do. We want to cook, sleep, build stuff, go outside, touch grass, whatever. If an agent is only “safe” when we supervise it 24/7, then it’s not really helping. It’s just adding a new job I didn't apply for.

I’ve always been into security and privacy, so I’ve never been comfortable giving software a bunch of power without guardrails. Still having a real agent that I can communicate with while I'm AFK was very exiting. The problem is how easily things can go sideways when software gets too much power. An agent that can help can also break stuff, and if it gets manipulated by someone, it can do the wrong thing fast. That whole problem was getting on my nerves, because I wanted to enjoy life and have some fun, not stress-test my patience.

So I decided to build an environment that makes agents feel safe enough to actually enjoy, and simple enough that other people can try it too. 
I came up with that design seamlessly easy to integrate it with the current popular solutions.

## The vibe

The Ruler is built around one simple idea:

Agents useful, nerd no babysitting, nerd afk, nerd smiling, nerd happy (Yes "nerd" includes you too &#59;))

So the default experience should feel like this:

- Local work should just work.
- The risky stuff should be clearly gated.
- When approval is needed, it should be quick and obvious.
- The agent shouldn’t crash or derail just because it’s waiting for approval.
- We should always be able to see what happened, without digging through chaos.

## What Agent Ruler does

The Ruler runs the agent inside a confined workspace and applies deterministic rules when the agent tries to cross real boundaries.

Think of it like a set of guardrails that are strict where they need to be, and chill where they can be.

It focuses on a few key boundaries:

- System critical files and settings
- Secrets and sensitive locations
- Outside world communication
- Delivery of outputs to real destinations
- Persistence-type behaviors that could turn into sneaky backdoors

Inside the workspace, the agent can work normally. But when it tries to cross a boundary, Agent Ruler either blocks it, stages it, or asks for approval.

No guessing. No vibes. Just rules and receipts.

## A simple picture of how it feels

This flow graph stays aligned with the current Control Panel behavior: boundary checks branch to allow, pending-approval, or deny, and every branch emits receipts for auditability.

<div style="display:flex; justify-content:center; width:100%;">
  <img
    src="/images/agent-ruler-approval-flow.svg"
    alt="Agent Ruler boundary and approval flow"
    style="max-width:900px; width:100%; height:auto;"
  />
</div>

## Why deterministic matters

Agent Ruler is not an LLM. It’s not trying to “interpret intent” or “decide safety” by guessing.

It’s deterministic on purpose because:

- We can test it.
- We can audit it.
- We can reproduce behavior.
- We can trust that the same input produces the same output.

That’s how you build something stable that people can actually rely on.

## What it’s not trying to be

Agent Ruler is not here to be magic or perfect.

It’s not:
- a full antivirus replacement
- a kernel-level security product
- a guarantee that nothing bad can ever happen

It is focused on something specific and practical:

Make agent capabilities governable and predictable, especially when untrusted outside content might try to steer the agent into doing something dumb.

## What you should expect

If you use Agent Ruler, you should expect:

- Local work to feel smooth.
- The system to block or stage risky actions.
- Approvals to be rare and meaningful.
- Clear receipts explaining what happened and why.
- A setup that keeps agents helpful without turning you into a full time supervisor.

If this sounds like your kind of thing, start with the Getting Started guide and run your agent inside the workspace. Then you can tighten or relax controls depending on what you need.

And if you ever catch yourself thinking “wait why is my agent trying to do that”, you’re exactly the kind of person I built this for.


## Where to go next

- Setup: [Getting Started](/guides/getting-started)
- Day-to-day UI operations: [Control Panel Guide](/guides/control-panel)
- Agent wiring: [OpenClaw Guide](/integrations/openclaw-guide)
