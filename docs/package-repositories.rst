.. _sysadmin_package_repositories:

Debian Package Repositories
---------------------------

All Debian based systems use APT_ as a package management tool. The lists of
repositories are defined in ``/etc/apt/sources.list`` and the ``.list`` files found
in the ``/etc/apt/sources.d/`` directory. Updates can be installed directly
with the ``apt`` command line tool, or via the GUI.

APT_ ``sources.list`` files list one package repository per line, with the most
preferred source listed first. Empty lines are ignored and a ``#`` character
anywhere on a line marks the remainder of that line as a comment. The
information available from the configured sources is acquired by ``apt
update``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://ftp.debian.org/debian buster main contrib
  deb http://ftp.debian.org/debian buster-updates main contrib

  # security updates
  deb http://security.debian.org/debian-security buster/updates main contrib


.. FIXME for 7.0: change security update suite to bullseye-security

In addition, you need a package repository from Proxmox to get Proxmox Backup
updates.

SecureApt
~~~~~~~~~

The `Release` files in the repositories are signed with GnuPG. APT is using
these signatures to verify that all packages are from a trusted source.

If you install Proxmox Backup Server from an official ISO image, the
verification key is already installed.

If you install Proxmox Backup Server on top of Debian, download and install the
key with the following commands:

.. code-block:: console

 # wget http://download.proxmox.com/debian/proxmox-ve-release-6.x.gpg -O /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg

Verify the SHA512 checksum afterwards with:

.. code-block:: console

 # sha512sum /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg

The output should be:

.. code-block:: console

 acca6f416917e8e11490a08a1e2842d500b3a5d9f322c6319db0927b2901c3eae23cfb5cd5df6facf2b57399d3cfa52ad7769ebdd75d9b204549ca147da52626 /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg

and the md5sum:

.. code-block:: console

 # md5sum /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg

Here, the output should be:

.. code-block:: console

 f3f6c5a3a67baf38ad178e5ff1ee270c /etc/apt/trusted.gpg.d/proxmox-ve-release-6.x.gpg

.. _sysadmin_package_repos_enterprise:

`Proxmox Backup`_ Enterprise Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This is the stable, recommended repository. It is available for
all `Proxmox Backup`_ subscription users. It contains the most stable packages,
and is suitable for production use. The ``pbs-enterprise`` repository is
enabled by default:

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list.d/pbs-enterprise.list``

  deb https://enterprise.proxmox.com/debian/pbs buster pbs-enterprise


To never miss important security fixes, the superuser (``root@pam`` user) is
notified via email about new packages as soon as they are available. The
change-log and details of each package can be viewed in the GUI (if available).

Please note that you need a valid subscription key to access this
repository. More information regarding subscription levels and pricing can be
found at https://www.proxmox.com/en/proxmox-backup-server/pricing

.. note:: You can disable this repository by commenting out the above line
 using a `#` (at the start of the line). This prevents error messages if you do
 not have a subscription key. Please configure the ``pbs-no-subscription``
 repository in that case.


`Proxmox Backup`_ No-Subscription Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

As the name suggests, you do not need a subscription key to access
this repository. It can be used for testing and non-production
use. It is not recommended to use it on production servers, because these
packages are not always heavily tested and validated.

We recommend to configure this repository in ``/etc/apt/sources.list``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://ftp.debian.org/debian buster main contrib
  deb http://ftp.debian.org/debian buster-updates main contrib

  # PBS pbs-no-subscription repository provided by proxmox.com,
  # NOT recommended for production use
  deb http://download.proxmox.com/debian/pbs buster pbs-no-subscription

  # security updates
  deb http://security.debian.org/debian-security buster/updates main contrib


`Proxmox Backup`_ Test Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This repository contains the latest packages and is heavily used by developers
to test new features.

.. .. warning:: the ``pbstest`` repository should (as the name implies)
  only be used to test new features or bug fixes.

You can access this repository by adding the following line to
``/etc/apt/sources.list``:

.. code-block:: sources.list
  :caption: sources.list entry for ``pbstest``

  deb http://download.proxmox.com/debian/pbs buster pbstest
