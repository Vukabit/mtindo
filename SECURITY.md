# Security

`mto` rewrites source files, so a bug that changes what code means, writes
outside the files it was given, or loses data is treated as a security
problem, as is anything in the release process that could ship a binary
other than the one built from this repository.

## Supported versions

Fixes go into the newest release. A serious problem in an older line still in
use is fixed there too, as a patch release.

## Reporting a problem

Report it privately: on the repository's **Security** tab, choose **Report a
vulnerability**. Please do not open a public issue for it.

Include what you ran, the `mto --version` line, and an input that shows the
problem; `mto report` writes a report with all of this. You will hear back
once the report has been read, and a fix is released as a patch release and
credited to you unless you would rather it were not.
