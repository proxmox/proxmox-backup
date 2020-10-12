``rustup`` Toolchain
====================

We normally want to build with the ``rustc`` Debian package. To do that
you can set the following ``rustup`` configuration:

    # rustup toolchain link system /usr
    # rustup default system


Versioning of proxmox helper crates
===================================

To use current git master code of the proxmox* helper crates, add::

   git = "git://git.proxmox.com/git/proxmox"

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


Build
=====
on Debian Buster

Setup:
  1. # echo 'deb http://download.proxmox.com/debian/devel/ buster main' >> /etc/apt/sources.list.d/proxmox-devel.list
  2. # sudo wget http://download.proxmox.com/debian/proxmox-ve-release-6.x.gpg -O /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg
  3. # sudo apt update
  4. # sudo apt install devscripts debcargo clang
  5. # git clone git://git.proxmox.com/git/proxmox-backup.git
  6. # sudo mk-build-deps -ir

Note: 2. may be skipped if you already added the PVE or PBS package repository

You are now able to build using the Makefile or cargo itself.
