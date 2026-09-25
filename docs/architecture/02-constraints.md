# Constraints

## Technical constraints

- **Implementation in Go:** The libraries we need to use for this tool exist 
  in go, so the application must also be written in Go if we want to get 
  maximum results.

- **Application must work on Mac/Linux:** We must ensure we cover enough 
  operating systems to reach the target audience for the application. However,
  due to limitations in the libraries we can use, we cannot cover Windows.
  However, we can use WSL2 for Windows support. As soon as nerdbox supports 
  Windows, we'll add Windows support in this tool as well.

- **Application runs without root permissions:** We must ensure that the 
  application doesn't need any root permissions on the host to limit the impact
  of a breach in the sandbox.

## Conventions

- **Coding conventions:** we follow the coding conventions published as part of
  [golangci-lint][LINTER] to ensure adequate formatting of the code.

- **Architecture documentation:** we use [Arc42][ARC42] style architecture 
  documentation. 

[LINTER]: https://golangci-lint.run/
[ARC42]: https://docs.arc42.org/

