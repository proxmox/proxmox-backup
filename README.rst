Versioning of proxmox helper crates
===================================

To use current git master code of the proxmox* helper crates, add::

   git = "ssh://gitolite3@proxdev.maurer-it.com/rust/proxmox"

to the proxmox dependency, and update the version to reflect the current,
pre-release version number (e.g., "0.1.1-dev.1" instead of "0.1.0").

Local (packaged) crates
=======================

To use locally installed, packaged crates instead of crates.io put the
following into ./.cargo/config (or point CARGO_HOME to a directory containing
such a config file)::

   [source]
   [source.debian-packages]
   directory = "/usr/share/cargo/registry"
   [source.crates-io]
   replace-with = "debian-packages"

This is akin to what happens when building with dh_cargo. Cargo.lock needs to
be deleted when switching between packaged crates and crates.io, since the
checksums are not compatible.
