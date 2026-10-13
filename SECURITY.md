# Security Policy

Zacess keeps people's documents, notes, recordings and reading profiles on their own devices. A flaw that exposes that material or lets a document run code is treated as urgent.

## Scope

In scope:

- A document, web page or file that runs code or reads data it should not when Zacess opens it, including through PDF, Word, HTML, image or audio import
- Cross-site scripting or unsafe link handling in the browser version or the exports
- The browser version sending a document, a reading profile or anything typed to another origin
- Data in the local library exposed to another user or app on the same device beyond what the operating system allows
- Secrets or personal data exposed by default configuration or logs

## Out of scope

- Attacks that need full control of the device or the person's account on it
- Vulnerabilities in third-party dependencies with no demonstrated impact on Zacess (report those upstream)

## Supported versions

Zacess is pre-release. Only the latest commit on `main` receives security fixes.

## Reporting a vulnerability

> [!IMPORTANT]
> Report privately, never in a public issue or discussion. Use [GitHub private vulnerability reporting](https://github.com/zacesshq/zacess/security/advisories/new) or email [contact@zacess.com](mailto:contact@zacess.com) with details and reproduction steps. Expect an acknowledgement within a few days.
