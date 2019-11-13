.. _sysadmin_package_repositories:

Debian Package Repositories
---------------------------

All Debian based systems use APT_ as package
management tool. The list of repositories is defined in
``/etc/apt/sources.list`` and ``.list`` files found inside
``/etc/apt/sources.d/``. Updates can be installed directly using
the ``apt`` command line tool, or via the GUI.

APT_ ``sources.list`` files list one package repository per line, with
the most preferred source listed first. Empty lines are ignored, and a
``#`` character anywhere on a line marks the remainder of that line as a
comment. The information available from the configured sources is
acquired by ``apt update``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``
	   
  deb http://ftp.debian.org/debian buster main contrib
  deb http://ftp.debian.org/debian buster-updates main contrib

  # security updates
  deb http://security.debian.org/debian-security buster/updates main contrib

  
.. FIXME for 7.0: change security update suite to bullseye-security

In addition, Proxmox provides three different package repositories for
the backup server binaries.

`Proxmox Backup`_ Enterprise Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This is the default, stable and recommended repository, available for
all `Proxmox Backup`_ subscription users. It contains the most stable packages,
and is suitable for production use. The ``pbs-enterprise`` repository is
enabled by default:

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list.d/pbs-enterprise.list``

  deb https://enterprise.proxmox.com/debian/pbs buster pbs-enterprise


As soon as updates are available, the superuser (``root@pam`` user) is
notified via email about the available new packages. On the GUI, the
change-log of each package can be viewed (if available), showing all
details of the update. So you will never miss important security
fixes.

Please note that you need a valid subscription key to access this
repository. We offer different support levels, and you can find further
details at https://www.proxmox.com/en/proxmox-backup/pricing.

.. note:: You can disable this repository by commenting out the above
  line using a `#` (at the start of the line). This prevents error
  messages if you do not have a subscription key. Please configure the
  ``pbs-no-subscription`` repository in that case.


`Proxmox Backup`_ No-Subscription Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

As the name suggests, you do not need a subscription key to access
this repository. It can be used for testing and non-production
use. Its not recommended to run on production servers, as these
packages are not always heavily tested and validated.

We recommend to configure this repository in ``/etc/apt/sources.list``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://ftp.debian.org/debian buster main contrib
  deb http://ftp.debian.org/debian buster-updates main contrib

  # PBS pbs-no-subscription repository provided by proxmox.com,
  # NOT recommended for production use
  deb http://download.proxmox.com/debian/bps buster pbs-no-subscription

  # security updates
  deb http://security.debian.org/debian-security buster/updates main contrib


`Proxmox Backup`_ Test Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Finally, there is a repository called ``pbstest``. This one contains the
latest packages and is heavily used by developers to test new
features.

.. warning:: the ``pbstest`` repository should (as the name implies)
  only be used for testing new features or bug fixes.

As usual, you can configure this using ``/etc/apt/sources.list`` by
adding the following line:

.. code-block:: sources.list
  :caption: sources.list entry for ``pbstest``

  deb http://download.proxmox.com/debian/bps buster pbstest

