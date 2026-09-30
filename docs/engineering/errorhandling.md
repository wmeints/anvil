# Working with errors in the code

This section covers how to handle various patterns around generating and consuming errors in the code.

## Generating errors

Always use a package-level sentinel to define a new error. You can return the sentinel directly if you don't need to 
append any additional information. Otherwise, use the `fmt.Errorf(...)` function to format the error with the required
additional information.

Example:

```go
package mypackage

import (
	"errors"
	"os"
	"fmt"
)

var ErrMissingFile = errors.New("file is missing")

func Load(path string) error {
	if _, err := os.Stat(path); err != nil {
		return fmt.Errorf("%w: %s (check the file path and permissions)", ErrMissingFile, path)
	}

	// ...
	
	return nil
}
```

## Enriching errors

It's often necessary to enrich raw errors returned by components we use because the original error contains too little
information to debug the problem. Use the following pattern to enrich errors:

```go
package mypackage

import (
	"errors"
	"os"
	"fmt"
)

var ErrMissingFile = errors.New("file is missing")

func Load(path string) error {
	if _, err := os.Stat(path); err != nil {
		return fmt.Errorf("%w: %s (check the file path and permissions) %w", ErrMissingFile, path, err)
	}

	// ...
	
	return nil
}
```

Notice that go allows you to use multiple `%w` placeholders. The first one should always contain the sentinel we created
in the package. The second placeholder can contain the original error produced by the underlying component.

## Custom errors

Prefer to use the `fmt.Errorf` when generating errors. However, when you need additional information in the caller, you 
can create a custom error with the following pattern. Return the error as a pointer and implement `Error()` on the
pointer receiver, so `errors.As` with a `*FileMissingError` target matches it:

```go
package mypackage

import (
	"os"
	"fmt"
	"errors"
)

type FileMissingError struct {
	Path string
}

func (err *FileMissingError) Error() string {
	return fmt.Sprintf("file is missing: %s (check file path and permissions)", err.Path)
}

func Load(path string) error {
	if _, err := os.Stat(path); err != nil {
		return &FileMissingError{
			Path: path,
		}
	}

	// ...
	
	return nil
}

func OtherFunction() {
	err := Load("some-file.txt")

	// Convert the error to its specific type, and extract the data.
	var missingFileErr *FileMissingError
	if errors.As(err, &missingFileErr) {
		fmt.Println("missing:", missingFileErr.Path)
	}
}
```