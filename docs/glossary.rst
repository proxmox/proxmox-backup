Glossary
========

.. glossary::

   `Virtual machine`_

      A virtual machine is a program that can execute an entire
      operating system inside an emulated hardware environment.

   `Container`_

      A container is an isolated user space. Programs run directly on
      the host's kernel, but with limited access to the host resources.

   Datastore

      A place to store backups. A directory which contains the backup data.
      The current implemenation is file-system based.

   `Rust`_

      Rust is a new, fast and memory-efficient system programming
      language. It has no runtime or garbage collector. Rust’s rich type
      system and ownership model guarantee memory-safety and
      thread-safety. I can eliminate many classes of bugs
      at compile-time.

   `Sphinx`_

      Is a tool that makes it easy to create intelligent and
      beautiful documentation. It was originally created for the
      documentation of the Python programming language. It has excellent facilities for the
      documentation of software projects in a range of languages.

   `reStructuredText`_

      Is an easy-to-read, what-you-see-is-what-you-get plaintext
      markup syntax and parser system.

   `FUSE`

      Filesystem in Userspace (`FUSE <https://en.wikipedia.org/wiki/Filesystem_in_Userspace>`_)
      defines an interface which makes it possible to implement a filesystem in
      userspace as opposed to implementing it in the kernel. The fuse
      kernel driver handles filesystem requests and sends them to a
      userspace application.

   Remote

      A remote Proxmox Backup Server installation and credentials for a user on it.
      You can pull datastores from a remote to a local datastore in order to
      have redundant backups.

   Schedule

      Certain tasks, for example pruning and garbage collection, need to be
      performed on a regular basis. Proxmox Backup Server uses a subset of the
      `systemd Time and Date Specification
      <https://www.freedesktop.org/software/systemd/man/systemd.time.html#>`_.
      The subset currently supports time of day specifications and weekdays, in
      addition to the shorthand expressions 'minutely', 'hourly', 'daily'.
      There is no support for specifying timezones, the tasks are run in the
      timezone configured on the server.
