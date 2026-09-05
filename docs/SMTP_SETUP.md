# SMTP setup

LaTeX Core has a provider-neutral SMTP client. Basic installation works with `LATEX_CORE_MAIL_ENABLED=false`; real delivery requires valid credentials from an external provider.

Configure it interactively without opening `.env`:

```sh
./install.sh --configure-mail
```

The helper updates only mail-related `.env` keys, hides the password, and validates the same security values as the server: `starttls` and `tls`.

| Variable | Meaning |
|---|---|
| `LATEX_CORE_MAIL_ENABLED` | `true` or `false` |
| `LATEX_CORE_SMTP_HOST` | SMTP server hostname |
| `LATEX_CORE_SMTP_PORT` | Port 1–65535 |
| `LATEX_CORE_SMTP_USERNAME` | Optional username; password must be set with it |
| `LATEX_CORE_SMTP_PASSWORD` | Optional SMTP password |
| `LATEX_CORE_SMTP_FROM_EMAIL` | Valid envelope/from address |
| `LATEX_CORE_SMTP_FROM_NAME` | Non-empty display name |
| `LATEX_CORE_SMTP_SECURITY` | Exactly `starttls` or `tls` |
| `LATEX_CORE_PUBLIC_BASE_URL` | Public `http://` or `https://` URL used in email |
| `LATEX_CORE_MAIL_SECRET_KEY` | Standard-base64 encoding of exactly 32 random bytes |
| `LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS` | 1–168; default 72 |
| `LATEX_CORE_MAIL_BATCH_SIZE` | 1–100; default 20 |
| `LATEX_CORE_MAIL_MAX_ATTEMPTS` | 1–20; default 5 |

Typical provider choices are university SMTP (`starttls`, often 587), Gmail with an App Password (`smtp.gmail.com`, `starttls`, 587), Amazon SES SMTP credentials (`starttls`, 587), and SendGrid SMTP (`smtp.sendgrid.net`, `starttls`, 587). Provider settings change; use the provider's current documentation. Never use an ordinary account password when an App Password or SMTP-specific credential is required.

After configuration:

```sh
./latex-core restart
./latex-core doctor
```

## Temporary-password delivery

A newly imported Student becomes a Writer; assigned Faculty becomes a Mentor. LaTeX Core generates a random temporary password, stores only its Argon2 hash, encrypts the one-time plaintext in the durable PostgreSQL outbox, sends it through SMTP, and erases the encrypted payload after delivery or expiry. First login requires a permanent 12–256 character password. The generated credentials CSV remains a one-time Admin fallback.

To test real delivery:

1. Configure working external SMTP credentials.
2. Replace the three demo addresses with inboxes you control.
3. Import the new Student/Faculty/Team demo records.
4. Confirm the temporary-password emails arrive.
5. Sign in with a temporary password.
6. Set a permanent password when prompted.

Real-provider delivery cannot be qualified without valid external SMTP credentials.
