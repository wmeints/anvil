# Introduction and goals

## Purpose of this project

This project provides an easy-to-use agent sandbox that doesn't require a 
license from some cloud provider or other commercial organization. This project
supports two major scenarios:

1. Terminal-based agents like [Claude Code](https://claude.ai/code), 
   [Oh-my-pi](https://omp.sh), and [OpenCode](https://opencode.ai) via 
   a CLI interface
   
2. IDE-integrated agents like GitHub Copilot provided that the IDE is connected 
   to the sandbox via SSH

## High level requirements

- Users can create sandboxes via dev containers that are then run like a VM.
- Users can control (start/stop/suspend/resume) sandboxes via the CLI.
- Users can connect to sandboxes via the CLI and via SSH.
- Users can control egress network traffic coming from the sandbox.
- Users can use secrets with the sandbox without exposing the secrets to the agent.

## Quality goals

- **Security:** Agents can safely run inside the sandbox without touching data on the host.
- **Security:** Agents cannot access outside website without express permission from the user.
- **Performance**: Sandboxes start within seconds so work can continue quickly.
- **Compatibility:** Sandboxes use OCI images to ensure developers can easily build sandboxes.
- **Reliability:** Sandboxes can be easily restarted and recreated should they fail.

## Stakeholders

We focus on software engineers working with agents on their laptops. This tool 
is not meant as something you run on a larger scale.

