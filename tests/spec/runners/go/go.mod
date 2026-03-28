module github.com/GrafeoDB/grafeo/tests/spec/runners/go

go 1.22

require (
	github.com/GrafeoDB/grafeo/crates/bindings/go v0.0.0
	gopkg.in/yaml.v3 v3.0.1
)

replace github.com/GrafeoDB/grafeo/crates/bindings/go => ../../../../crates/bindings/go
