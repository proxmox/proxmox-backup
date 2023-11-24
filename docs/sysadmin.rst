.. _sysadmin_host_administration:

Host System Administration
==========================

`Proxmox Backup`_ is based on the famous Debian_ Linux
distribution. This means that you have access to the entire range of
Debian packages, and that the base system is well documented. The `Debian
Administrator's Handbook`_ is available online, and provides a
comprehensive introduction to the Debian operating system.

A standard Proxmox Backup installation uses the default
repositories from Debian, so you get bug fixes and security updates
through that channel. In addition, we provide our own package
repository to roll out all Proxmox related packages. This includes
updates to some Debian packages when necessary.

We also deliver a specially optimized Linux kernel, based on the Ubuntu
kernel. This kernel includes drivers for ZFS_.

The following sections will concentrate on backup related topics. They
will explain things which are different on Proxmox Backup, or
tasks which are commonly used on Proxmox Backup. For other topics,
please refer to the standard Debian documentation.


.. include:: local-zfs.rst

.. include:: system-booting.rst

.. include:: certificate-management.rst

.. include:: services.rst

.. include:: command-line-tools.rst
