FAQ
===

What distribution is Proxmox Backup Server (PBS) based on?
----------------------------------------------------------

Proxmox Backup Server is based on `Debian GNU/Linux <https://www.debian.org/>`_.


Which platforms are supported as a backup source (client)?
----------------------------------------------------------

The client tool works on most modern Linux systems, meaning you are not limited
to Debian-based backups.


Will Proxmox Backup Server run on a 32-bit processor?
-----------------------------------------------------

Proxmox Backup Server only supports 64-bit CPUs (AMD or Intel). There are no
future plans to support 32-bit processors.


How long will my Proxmox Backup Server version be supported?
------------------------------------------------------------

+-----------------------+----------------------+---------------+------------+--------------------+
|Proxmox Backup Version | Debian Version       | First Release | Debian EOL | Proxmox Backup EOL |
+=======================+======================+===============+============+====================+
|Proxmox Backup 2.x     | Debian 11 (Bullseye) | 2021-07       | tba        | tba                |
+-----------------------+----------------------+---------------+------------+--------------------+
|Proxmox Backup 1.x     | Debian 10 (Buster)   | 2020-11       | ~Q2/2022   | Q2-Q3/2022         |
+-----------------------+----------------------+---------------+------------+--------------------+


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


Is the backup incremental/deduplicated?
---------------------------------------

With Proxmox Backup Server, backups are sent incremental and data is
deduplicated on the server.
This minimizes both the storage consumed and the network impact.
