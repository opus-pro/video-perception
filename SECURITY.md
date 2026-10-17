# Security policy

Please do not open a public issue for a suspected vulnerability. Use GitHub's private vulnerability reporting for `opus-pro/video-perception` when available, or contact the repository owner privately through GitHub.

Include the affected version, reproduction steps, impact, and any suggested mitigation. Do not include real credentials, private media, or personal data.

The project supports the latest published release. Video Perception processes untrusted media through local tools and optional providers; reports involving command injection, path traversal, credential exposure, unsafe archive handling, or unexpected remote media transfer are especially important.

## Telemetry and network use

Video Perception sends no telemetry. The CLI, the MCP server, and the plugin make network requests in exactly two cases, both chosen by the user: `update`, `install.sh`, and the npm launcher fetch release assets from GitHub and verify them against the published `checksums.txt`; and a remote provider receives media only when a SensePlan or strategy names it. Discovery commands never read credentials or contact a service, and `doctor` probes only the local services it reports. A build that adds any other network request is a bug to report here.
