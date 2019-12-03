Installation
============

`Proxmox Backup`_ is split into a server part and a client part. The
server part comes with it's own graphical installer, but we also
ship Debian_ package repositories, so you can easily install those
packages on any Debian_ based system.

.. include:: package-repositories.rst


Server installation
-------------------

The backup server stores the actual backup data, but also provides a
web based GUI for various management tasks, for example disk
management.

.. note:: You always need a backup server. It is not possible to use
   `Proxmox Backup`_ without the server part.

The server is based on Debian, therefore the disk image (ISO file) provided
by us includes a complete Debian system ("buster" for version 1.x) as
well as all necessary backup packages.

Using the installer will guide you through the setup, allowing
you to partition the local disk(s), apply basic system configurations
(e.g. timezone, language, network) and install all required packages.
Using the provided ISO will get you started in just a few minutes,
that's why we recommend this method for new and existing users.

Alternatively, `Proxmox Backup`_ server can be installed on top of an
existing Debian system.

Using the `Proxmox Backup`_ Installer
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You can download the ISO from |DOWNLOADS|.
It includes the following:

* Complete operating system (Debian Linux, 64-bit)

* The `Proxmox Backup`_ server installer, which partitions the local
  disk(s) with ext4, ext3, xfs or ZFS and installs the operating
  system.

* Our Linux kernel with ZFS support.

* Complete toolset for administering backups and all necessary
  resources

* Web based management interface for using the toolset

.. note:: During the installation process, the complete server
   is used by default and all existing data is removed.


Install `Proxmox Backup`_ server on Debian
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Proxmox ships as a set of Debian packages, so you can install it on
top of a standard Debian installation.  After configuring the
:ref:`sysadmin_package_repositories`, you need to run:

.. code-block:: console

  # apt-get update
  # apt-get install proxmox-backup-server

Above code keeps the current (Debian) kernel and installs a minimal
set of required packages.

If you want to install the same set of packages as the installer
does, please use the following:

.. code-block:: console

  # apt-get update
  # apt-get install proxmox-backup

This installs all required packages, the Proxmox kernel with ZFS_
support, and a set of commonly useful packages.

Installing on top of an existing Debian_ installation looks easy, but
it presumes that you have correctly installed the base system, and you
know how you want to configure and use the local storage. Network
configuration is also completely up to you.

In general, this is not trivial, especially when you use LVM_ or
ZFS_.

Install Proxmox Backup server on `Proxmox VE`_
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

After configuring the
:ref:`sysadmin_package_repositories`, you need to run:

.. code-block:: console

  # apt-get update
  # apt-get install proxmox-backup-server

.. caution:: Installing the backup server directly on the hypervisor
   is not recommended. It is more secure to use a separate physical
   server to store backups. If the hypervisor server fails, you can
   still access your backups.

Client installation
-------------------

Install `Proxmox Backup`_ client on Debian
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Proxmox ships as a set of Debian packages, so you can install it on
top of a standard Debian installation.  After configuring the
:ref:`sysadmin_package_repositories`, you need to run:

.. code-block:: console

  # apt-get update
  # apt-get install proxmox-backup-client


Installing from source
~~~~~~~~~~~~~~~~~~~~~~

.. todo:: Add section "Installing from source"

Installing statically linked binary
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. todo:: Add section "Installing statically linked binary"
