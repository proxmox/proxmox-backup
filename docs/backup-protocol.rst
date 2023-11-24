Backup Protocol
===============

`Proxmox Backup`_ Server uses a REST-based API. While the management
interface uses normal HTTP, the actual backup and restore interface uses
HTTP/2 for improved performance. Both HTTP and HTTP/2 are well known
standards, so the following section assumes that you are familiar with
how to use them.


Backup Protocol API
-------------------

To start a new backup, the API call ``GET /api2/json/backup`` needs to
be upgraded to a HTTP/2 connection using
``proxmox-backup-protocol-v1`` as the protocol name::

  GET /api2/json/backup HTTP/1.1
  UPGRADE: proxmox-backup-protocol-v1

The server replies with the ``HTTP 101 Switching Protocol`` status code,
and you can then issue REST commands on the updated HTTP/2 connection.

The backup protocol allows you to upload three different kind of files:

- Chunks and blobs (binary data)

- Fixed indexes (List of chunks with fixed size)

- Dynamic indexes (List of chunks with variable size)

The following section provides a short introduction on how to upload such
files. Please use the `API Viewer <api-viewer/index.html>`_ for
details about the available REST commands.


Upload Blobs
~~~~~~~~~~~~

Blobs are uploaded using ``POST /blob``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`.

The file name must end with ``.blob``, and is automatically added
to the backup manifest, following the call to ``POST /finish``.


Upload Chunks
~~~~~~~~~~~~~

Chunks belong to an index, so you first need to open an index (see
below). After that, you can upload chunks using ``POST /fixed_chunk``
and ``POST /dynamic_chunk``. The HTTP body contains the chunk data
encoded as :ref:`Data Blob <data-blob-format>`).


Upload Fixed Indexes
~~~~~~~~~~~~~~~~~~~~

Fixed indexes are used to store VM image data. The VM image is split
into equally sized chunks, which are uploaded individually. The index
file simply contains a list of chunk digests.

You create a fixed index with ``POST /fixed_index``. Then, upload
chunks with ``POST /fixed_chunk``, and append them to the index with
``PUT /fixed_index``. When finished, you need to close the index using
``POST /fixed_close``.

The file name needs to end with ``.fidx``, and is automatically added
to the backup manifest, following the call to ``POST /finish``.


Upload Dynamic Indexes
~~~~~~~~~~~~~~~~~~~~~~

Dynamic indexes are used to store file archive data. The archive data
is split into dynamically sized chunks, which are uploaded
individually. The index file simply contains a list of chunk digests
and offsets.

You can create a dynamically sized index with ``POST /dynamic_index``. Then,
upload chunks with ``POST /dynamic_chunk``, and append them to the index with
``PUT /dynamic_index``. When finished, you need to close the index using
``POST /dynamic_close``.

The filename needs to end with ``.didx``, and is automatically added
to the backup manifest, following the call to ``POST /finish``.


Finish Backup
~~~~~~~~~~~~~

Once you have uploaded all data, you need to call ``POST /finish``. This
commits all data and ends the backup protocol.


Restore/Reader Protocol API
---------------------------

To start a new reader, the API call ``GET /api2/json/reader`` needs to
be upgraded to a HTTP/2 connection using
``proxmox-backup-reader-protocol-v1`` as protocol name::

  GET /api2/json/reader HTTP/1.1
  UPGRADE: proxmox-backup-reader-protocol-v1

The server replies with the ``HTTP 101 Switching Protocol`` status code,
and you can then issue REST commands on that updated HTTP/2 connection.

The reader protocol allows you to download three different kinds of files:

- Chunks and blobs (binary data)

- Fixed indexes (list of chunks with fixed size)

- Dynamic indexes (list of chunks with variable size)

The following section provides a short introduction on how to download such
files. Please use the `API Viewer <api-viewer/index.html>`_ for details about
the available REST commands.


Download Blobs
~~~~~~~~~~~~~~

Blobs are downloaded using ``GET /download``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`.


Download Chunks
~~~~~~~~~~~~~~~

Chunks are downloaded using ``GET /chunk``. The HTTP body contains the
data encoded as :ref:`Data Blob <data-blob-format>`.


Download Index Files
~~~~~~~~~~~~~~~~~~~~

Index files are downloaded using ``GET /download``. The HTTP body
contains the data encoded as :ref:`Fixed Index <fixed-index-format>`
or :ref:`Dynamic Index <dynamic-index-format>`.
