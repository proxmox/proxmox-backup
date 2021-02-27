Backup Protocol
===============

Proxmox Backup Server uses a REST based API. While the management
interface use normal HTTP, the actual backup and restore interface use
HTTP/2 for improved performance. Both HTTP and HTTP/2 are well known
standards, so the following section assumes that you are familiar on
how to use them.


Backup Protocol API
-------------------

To start a new backup, the API call ``GET /api2/json/backup`` needs to
be upgraded to a HTTP/2 connection using
``proxmox-backup-protocol-v1`` as protocol name::

  GET /api2/json/backup HTTP/1.1
  UPGRADE: proxmox-backup-protocol-v1

The server replies with HTTP 101 Switching Protocol status code,
and you can then issue REST command on that updated HTTP/2 connection.

Please use the `API Viewer <api-viewer/index.html>`_ for details about
available REST commands.


Restore/Reader Protocol API
---------------------------

To start a new reader, the API call ``GET /api2/json/reader`` needs to
be upgraded to a HTTP/2 connection using
``proxmox-backup-reader-protocol-v1`` as protocol name::

  GET /api2/json/reader HTTP/1.1
  UPGRADE: proxmox-backup-reader-protocol-v1

The server replies with HTTP 101 Switching Protocol status code,
and you can then issue REST command on that updated HTTP/2 connection.

Please use the `API Viewer <api-viewer/index.html>`_ for details about
available REST commands.
