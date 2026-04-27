# Avail Port
## Introduction
This is a port of Avail (described below) to use the privacy policy system [Sesame](https://github.com/brownsys/sesame).
The design of this port, the policies chosen, and the implementation of those policies are based on a version of Avail used to evaluate [Carapace](https://github.com/PLaSSticity/Carapace-implementation).
The basic intent of the policy is as follows: Calendars are private to their owners, and events inherit the privacy policy of their calendar.
Sesame related changes to this program are concentrated in the src/commands.rs and src/ifc.rs files.

## Changes to Sesame
Creating this port required changes to the current version of Sesame and Scrutinizer published on the brownsys GitHub.
The primary changes to Sesame and Scrutinizer revolved around finishing the API changes from Alohamora to Sesame.
Significant work also had to be done to manage dependencies that were no longer valid and inconsistent between Scrutinizer, Sesame, and dylint.
The only meaningful changes beyond dependency/build updates were to Sesame's lints/src/sesame_pcr.py and Scrutinizer's scrutils/src/body_cache/mod.rs.
In both of these cases the changes were meant to fix panics or compilation errors stemming from MIR and do not appear to have changed the intended behavior of the tools.

## Unfinished Work
This port does not implement any sort of rigorous performance testing or documentation of the changes made to Avail (runtime, added LoC, added files).
This port also erred on the side of similarity with the Carapace implementation of security and declassification, specifically with respect to critical regions.
Should a user wish to modify the implementation of critical regions to be more true to the nature of Sesame, the changes required should be minimal (wrapping outputs rather than declassifying).
This port also does not implement sandboxing for Sesame's critical regions. The dockerfile also reports an invalid signature that is valid locally, but accepts another signature.

## Running this Port
This port should work on Ubuntu 24, but still has incompatible dependencies on Windows systems. A dockerfile has been included and is the intended way to run this port.
To execute the dockerfile run `./scripts/docker-build.sh` to build the image, and once the build is complete run it with `docker run --rm avail:latest`.
To validate that Scrutinizer and Sesame's lints run correctly, run `./scripts/docker-build.sh &> build.log`.
The build is expected to take ~6–8 minutes, and any warnings or errors from either tool should be visible at the end of step #23.