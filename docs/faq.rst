FAQ
===

What distribution is Proxmox Backup Server (PBS) based on?
----------------------------------------------------------

`Proxmox Backup`_ Server is based on `Debian GNU/Linux <https://www.debian.org/>`_.


Which platforms are supported as a backup source (client)?
----------------------------------------------------------

The client tool works on most modern Linux systems, meaning you are not limited
to Debian-based backups.


Will Proxmox Backup Server run on a 32-bit processor?
-----------------------------------------------------

Proxmox Backup Server only supports 64-bit CPUs (AMD or Intel). There are no
future plans to support 32-bit processors.


.. _faq-support-table:

How long will my Proxmox Backup Server version be supported?
------------------------------------------------------------

.. csv-table::
   :file: faq-release-support-table.csv
   :widths: 30 26 13 13 18
   :header-rows: 1

How can I upgrade Proxmox Backup Server to the next point release?
------------------------------------------------------------------

Minor version upgrades, for example upgrading from Proxmox Backup Server in
version 3.1 to 3.2 or 3.3, can be done just like any normal update.
But, you should still check the `release notes
<https://pbs.proxmox.com/wiki/index.php/Roadmap>`_ for any relevant notable,
or breaking change.

For the update itself use either the Web UI *Node -> Updates* panel or
through the CLI with:

.. code-block:: console

  apt update
  apt full-upgrade

.. note:: Always ensure you correctly setup the
   :ref:`package repositories <sysadmin_package_repositories>` and only
   continue with the actual upgrade if `apt update` did not hit any error.

.. _faq-upgrade-major:

How can I upgrade Proxmox Backup Server to the next major release?
------------------------------------------------------------------

Major version upgrades, for example going from Proxmox Backup Server 2.4 to
3.1, are also supported.
They must be carefully planned and tested and should **never** be started
without having an off-site copy of the important backups, e.g., via remote sync
or tape, ready.

Although the specific upgrade steps depend on your respective setup, we provide
general instructions and advice of how a upgrade should be performed:

* `Upgrade from Proxmox Backup Server 2 to 3 <https://pbs.proxmox.com/wiki/index.php/Upgrade_from_2_to_3>`_

* `Upgrade from Proxmox Backup Server 1 to 2 <https://pbs.proxmox.com/wiki/index.php/Upgrade_from_1.1_to_2.x>`_

Can I copy or synchronize my datastore to another location?
-----------------------------------------------------------

Proxmox Backup Server allows you to copy or synchronize datastores to other
locations, through the use of *Remotes* and *Sync Jobs*. *Remote* is the term
given to a separate server, which has a datastore that can be synced to a local store.
A *Sync Job* is the process which is used to pull the contents of a datastore from
a *Remote* to a local datastore.


Can Proxmox Backup Server verify data integrity of a backup archive?
--------------------------------------------------------------------

Proxmox Backup Server uses a built-in SHA-256 checksum algorithm, to ensure
data integrity. Within each backup, a manifest file (index.json) is created,
which contains a list of all the backup files, along with their sizes and
checksums. This manifest file is used to verify the integrity of each backup.


When backing up to remote servers, do I have to trust the remote server?
------------------------------------------------------------------------

Proxmox Backup Server transfers data via `Transport Layer Security (TLS)
<https://en.wikipedia.org/wiki/Transport_Layer_Security>`_ and additionally
supports client-side encryption. This means that data is transferred securely
and can be encrypted before it reaches the server.  Thus, in the event that an
attacker gains access to the server or any point of the network, they will not
be able to read the data.

.. note:: Encryption is not enabled by default. To set up encryption, see the
  :ref:`backup client encryption section <client_encryption>`.


Is the backup incremental/deduplicated/full?
--------------------------------------------

With Proxmox Backup Server, backups are sent incrementally to the server, and
data is then deduplicated on the server. This minimizes both the storage
consumed and the impact on the network. Each backup still references all
data and such is a full backup. For details see the
:ref:`Technical Overview <tech_design_overview>`

.. todo:: document our stability guarantees, i.e., the separate one for, in
   increasing duration of how long we'll support it: api compat, backup
   protocol compat and backup format compat
