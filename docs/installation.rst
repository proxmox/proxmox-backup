Installation
============

`Proxmox Backup`_ is split into a server and client part. The server part
can either be installed with a graphical installer or on top of
Debian_ from the provided package repository.

.. include:: system-requirements.rst

.. include:: package-repositories.rst

Server Installation
-------------------

The backup server stores the actual backed up data and provides a web based GUI
for various management tasks such as disk management.

.. note:: You always need a backup server. It is not possible to use
   Proxmox Backup without the server part.

The disk image (ISO file) provided by Proxmox includes a complete Debian system
as well as all necessary packages for the Proxmox Backup Server.

The installer will guide you through the setup process and allow
you to partition the local disk(s), apply basic system configuration
(for example timezone, language, network), and install all required packages.
The provided ISO will get you started in just a few minutes, and is the
recommended method for new and existing users.

Alternatively, Proxmox Backup Server can be installed on top of an
existing Debian system.

Install `Proxmox Backup`_ Server using the Installer
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Download the ISO from |DOWNLOADS|.
It includes the following:

* The Proxmox Backup Server installer, which partitions the local
  disk(s) with ext4, xfs or ZFS, and installs the operating system

* Complete operating system (Debian Linux, 64-bit)

* Proxmox Linux kernel with ZFS support

* Complete tool-set to administer backups and all necessary resources

* Web based management interface

.. note:: During the installation process, the complete server
   is used by default and all existing data is removed.


Install `Proxmox Backup`_ Server on Debian
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Proxmox ships as a set of Debian packages which can be installed on top of a
standard Debian installation. After configuring the
:ref:`sysadmin_package_repositories`, you need to run:

.. code-block:: console

  # apt update
  # apt install proxmox-backup-server

The above commands keep the current (Debian) kernel and install a minimal
set of required packages.

If you want to install the same set of packages as the installer
does, please use the following:

.. code-block:: console

  # apt update
  # apt install proxmox-backup

This will install all required packages, the Proxmox kernel with ZFS_
support, and a set of common and useful packages.

.. caution:: Installing Proxmox Backup on top of an existing Debian_
  installation looks easy, but it assumes that the base system and local
  storage have been set up correctly. In general this is not trivial, especially
  when LVM_ or ZFS_ is used. The network configuration is completely up to you
  as well.

.. Note:: You can access the web interface of the Proxmox Backup Server with
   your web browser, using HTTPS on port 8007. For example at
   ``https://<ip-or-dns-name>:8007``

Install Proxmox Backup Server on `Proxmox VE`_
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

After configuring the
:ref:`sysadmin_package_repositories`, you need to run:

.. code-block:: console

  # apt update
  # apt install proxmox-backup-server

.. caution:: Installing the backup server directly on the hypervisor
   is not recommended. It is safer to use a separate physical
   server to store backups. Should the hypervisor server fail, you can
   still access the backups.

.. Note:: You can access the web interface of the Proxmox Backup Server with
   your web browser, using HTTPS on port 8007. For example at
   ``https://<ip-or-dns-name>:8007``

Client Installation
-------------------

Install Proxmox Backup Client on Debian
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Proxmox ships as a set of Debian packages to be installed on top of a standard
Debian installation. After configuring the :ref:`package_repositories_client_only_apt`,
you need to run:

.. code-block:: console

  # apt update
  # apt install proxmox-backup-client


.. note:: The client-only repository should be usable by most recent Debian and
   Ubuntu derivatives.

