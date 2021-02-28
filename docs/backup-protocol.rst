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
and you can then issue REST commands on that updated HTTP/2 connection.

The backup protocol allows you to upload three different kind of files:

- Chunks and blobs (binary data)

- Fixed Indexes (List of chunks with fixed size)

- Dynamic Indexes (List of chunk with variable size)

The following section gives a short introduction how to upload such
files. Please use the `API Viewer <api-viewer/index.html>`_ for
details about available REST commands.


Upload Blobs
~~~~~~~~~~~~

Uploading blobs is done using ``POST /blob``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`).

The file name needs to end with ``.blob``, and is automatically added
to the backup manifest.


Upload Chunks
~~~~~~~~~~~~~

Chunks belong to an index, so you first need to open an index (see
below). After that, you can upload chunks using ``POST /fixed_chunk``
and ``POST /dynamic_chunk``. The HTTP body contains the chunk data
encoded as :ref:`Data Blob <data-blob-format>`).


Upload Fixed Indexes
~~~~~~~~~~~~~~~~~~~~

Fixed indexes are use to store VM image data. The VM image is split
into equally sized chunks, which are uploaded individually. The index
file simply contains a list to chunk digests.

You create a fixed index with ``POST /fixed_index``. Then upload
chunks with ``POST /fixed_chunk``, and append them to the index with
``PUT /fixed_index``. When finished, you need to close the index using
``POST /fixed_close``.

The file name needs to end with ``.fidx``, and is automatically added
to the backup manifest.


Upload Dynamic Indexes
~~~~~~~~~~~~~~~~~~~~~~

Dynamic indexes are use to store file archive data. The archive data
is split into dynamically sized chunks, which are uploaded
individually. The index file simply contains a list to chunk digests
and offsets.

You create a dynamic sized index with ``POST /dynamic_index``. Then
upload chunks with ``POST /dynamic_chunk``, and append them to the index with
``PUT /dynamic_index``. When finished, you need to close the index using
``POST /dynamic_close``.

The file name needs to end with ``.didx``, and is automatically added
to the backup manifest.

Finish Backup
~~~~~~~~~~~~~

Once you have uploaded all data, you need to call ``POST
/finish``. This commits all data and ends the backup protocol.


Restore/Reader Protocol API
---------------------------

To start a new reader, the API call ``GET /api2/json/reader`` needs to
be upgraded to a HTTP/2 connection using
``proxmox-backup-reader-protocol-v1`` as protocol name::

  GET /api2/json/reader HTTP/1.1
  UPGRADE: proxmox-backup-reader-protocol-v1

The server replies with HTTP 101 Switching Protocol status code,
and you can then issue REST commands on that updated HTTP/2 connection.

The reader protocol allows you to download three different kind of files:

- Chunks and blobs (binary data)

- Fixed Indexes (List of chunks with fixed size)

- Dynamic Indexes (List of chunk with variable size)

The following section gives a short introduction how to download such
files. Please use the `API Viewer <api-viewer/index.html>`_ for details about
available REST commands.


Download Blobs
~~~~~~~~~~~~~~

Downloading blobs is done using ``GET /download``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`.


Download Chunks
~~~~~~~~~~~~~~~

Downloading chunks is done using ``GET /chunk``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`).


Download Index Files
~~~~~~~~~~~~~~~~~~~~

Downloading index files is done using ``GET /download``. The HTTP body
contains the data encoded as :ref:`Fixed Index <fixed-index-format>`
or :ref:`Dynamic Index <dynamic-index-format>`.
