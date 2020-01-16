``rustup`` Toolchain
====================

We normally want to build with the ``rustc`` Debian package. To do that
you can set the following ``rustup`` configuration:

    # rustup toolchain link system /usr
    # rustup default system


Versioning of proxmox helper crates
===================================

To use current git master code of the proxmox* helper crates, add::

   git = "ssh://gitolite3@proxdev.maurer-it.com/rust/proxmox"

or::

   path = "../proxmox/proxmox"

to the proxmox dependency, and update the version to reflect the current,
pre-release version number (e.g., "0.1.1-dev.1" instead of "0.1.0").

Local cargo config
==================

This repository ships with a ``.cargo/config`` that replaces the crates.io
registry with packaged crates located in ``/usr/share/cargo/registry``.

A similar config is also applied building with dh_cargo. Cargo.lock needs to be
deleted when switching between packaged crates and crates.io, since the
checksums are not compatible.

To reference new dependencies (or updated versions) that are not yet packaged,
the dependency needs to point directly to a path or git source (e.g., see
example for proxmox crate above).
