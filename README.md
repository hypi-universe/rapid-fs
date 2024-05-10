# RAPID Virtual Filesystem

This package is a simple library for providing a virtual file system API to Hypi's RAPID server.
It is not generic and likely doesn't suit any use case beyond Hypi's.

## What does it do?

A multi-tenant server which allows users to specify paths needs a way to prevent them from using `..` and so on to go to directories outside their own.
This ensures all access is within their folder or its sub-tree whilst also providing some APIs that make things more convenient for RAPID server's use case.
