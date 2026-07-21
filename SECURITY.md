# Security policy

## Reporting a vulnerability

Report privately first. Do not open a public issue for a security problem: Brisvia is a live cryptocurrency
and a public report gives an attacker the same head start it gives us.

Use GitHub's private reporting on this repository — **Security → Report a vulnerability** — or write to
**security@brisvia.com**.

Please include what you need to make the problem reproducible: affected version, operating system, the steps
you took, and what you expected versus what happened. A proof of concept helps, even a rough one.

We will acknowledge your report within 72 hours and tell you whether we can reproduce it. If the problem is
real, we will agree a disclosure date with you and credit you when the fix ships, unless you prefer
otherwise. There is no bug bounty programme: this is a project without funding, and we would rather say so
than imply a reward that does not exist.

## What is in scope

- The desktop app in this repository: wallet handling, key derivation, the update mechanism, the way the
  bundled node and mining engine are launched and supervised.
- The release and signing pipeline in `.github/workflows/`.
- Anything that could expose a user's 12 words, private keys or wallet file.

Consensus, networking and validation belong to the core repository:
[github.com/brisvia/brisvia](https://github.com/brisvia/brisvia). Report those there.

## What is not a vulnerability

- **Antivirus or SmartScreen warnings.** The executables are not code-signed yet, and the app genuinely
  bundles a CPU miner. See [brisvia.com/blocked](https://brisvia.com/blocked/).
- **Losing your 12 words.** Nobody can recover them. That is the design, not a defect.
- Reports produced only by an automated scanner, with no explanation of actual impact.

## Protecting yourself

- Your 12 words are your money. Anyone who has them has your coins.
- **No one from Brisvia will ever ask for your 12 words**, in any channel, for any reason. There are no
  giveaways and no airdrops. Every message that says otherwise is a scam.
- Download only from [brisvia.com](https://brisvia.com) or this repository's Releases page, and check the
  published SHA-256 before running the file.
- The app never sends your words, private keys or seed anywhere. If a build appears to, that is a
  vulnerability and we want to hear about it immediately.
