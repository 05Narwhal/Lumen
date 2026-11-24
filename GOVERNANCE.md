# Governance

Lumen is a small project and this is a short document.

## Who decides

**Julian (JL Studios) is the creator and lead maintainer and has the final say.** He tries to decide in the open: in issues and in GitHub Discussions, with the reasoning written down. Disagreement is welcome there; a decision that goes against you still gets an explanation.

The project's direction is in [docs/VISION.md](docs/VISION.md): local-first, no account, no cloud, no telemetry, no paid tier, free software. Proposals that change that are not accepted; everything else is open for discussion.

## Maintainers

Maintainers can review and merge pull requests. They are invited by the lead maintainer after a period of sustained, careful reviews and contributions. A maintainer who has been away for a long time may be moved to *emeritus* with thanks. Maintainers follow the [Code of Conduct](CODE_OF_CONDUCT.md) and the rules in [CONTRIBUTING.md](CONTRIBUTING.md).

## How work is decided

- Small changes: a pull request and a review.
- Larger work: an issue or Discussion first, then a short written plan, then the code. A decision that contradicts the architecture spine ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), §3) changes that document first.
- Releases are cut by the lead maintainer from `main` with a version tag; the release workflow builds and publishes them.

## Money

There is no paid tier of any kind. GitHub Sponsors donations are optional and are the only money the project takes.

## If Julian steps away

The project must keep working without one person. These are the arrangements:

- **One updater key.** Tauri accepts a single updater public key, so there is no second key to hand out. The key and its password are in three places (GitHub secrets, a password-manager entry, two offline copies); `docs/RELEASING.md` (*The signing key*) lists them. None of it is in this repository.
- **GitHub account successor.** Julian names a successor for the `05Narwhal` account in GitHub's settings. The successor may archive or transfer the repositories (`lumen`, `lumen-models`, `lumen-plugins`).
- **A sealed escrow.** The offline copy of the key files plus written instructions are held by a person Julian names in his private notes (`CLAUDE.local.md`), not here. With them the successor transfers the repositories to a GitHub organisation, adds the secrets and releases.
- **Installed apps follow a transfer.** GitHub redirects the old repository address after a transfer, and installed apps poll `https://github.com/05Narwhal/lumen/releases/download/channels/`. Never create a new repository under an old name after a transfer, because that breaks the redirect.
- A personal account gives collaborators write access only, with no triage or maintain roles; that is why a transfer to an organisation is the first thing to arrange if the project grows.
- If none of this has happened and installed apps stop receiving updates, the community can fork under another name (see [TRADEMARKS.md](TRADEMARKS.md)): the code is GPL, and a fork needs its own updater key and endpoint.
