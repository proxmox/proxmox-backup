Tape Backup
===========

Our tape backup solution provides a easy way to store datastore
contents on a tape. This increases data safety because you get:

- an additional copy of the data
- to a different media type (tape)
- to an additional location (you can move tape offsite)

Tape backups do not provide random access to the stored
data. Instead, you need to restore the data to disk before you can
access it again.


Tape Technology Primer
----------------------

.. _Linear Tape Open: https://en.wikipedia.org/wiki/Linear_Tape-Open

As of 2021, the only broadly available tape technology standard is
`Linear Tape Open`_, and different vendors offers LTO Ultrium tape
drives and autoloaders.

In general, LTO tapes offer the following advantages:

- Durable (30 years)
- High Capacity (12 TB)
- Relatively low cost per TB
- Cold Media
- Movable (storable inside vault)
- Multiple vendors (for both media and drives)
