.. _calendar-event-scheduling:

Calendar Events
===============

Introduction and Format
-----------------------

Certain tasks, for example pruning and garbage collection, need to be
performed on a regular basis. `Proxmox Backup`_ Server uses a format inspired
by the systemd Time and Date Specification (see `systemd.time manpage`_)
called `calendar events` for its schedules.

`Calendar events` are expressions to specify one or more points in time.
They are mostly compatible with systemd's calendar events.

The general format is as follows:

.. code-block:: console
  :caption: Calendar event

  [WEEKDAY] [[YEARS-]MONTHS-DAYS] [HOURS:MINUTES[:SECONDS]]

Note that there either has to be at least a weekday, date or time part.
If the weekday or date part is omitted, all (week)days are included.
If the time part is omitted, the time 00:00:00 is implied.
(e.g. '2020-01-01' refers to '2020-01-01 00:00:00')

Weekdays are specified with the abbreviated English version:
`mon, tue, wed, thu, fri, sat, sun`.

Each field can contain multiple values in the following formats:

* comma-separated: e.g.,  01,02,03
* as a range: e.g., 01..10
* as a repetition: e.g, 05/10 (means starting at 5 every 10)
* and a combination of the above: e.g., 01,05..10,12/02
* or a `*` for every possible value: e.g., \*:00

There are some special values that have a specific meaning:

=================================  ==============================
Value                              Syntax
=================================  ==============================
`minutely`                         `*-*-* *:*:00`
`hourly`                           `*-*-* *:00:00`
`daily`                            `*-*-* 00:00:00`
`weekly`                           `mon *-*-* 00:00:00`
`monthly`                          `*-*-01 00:00:00`
`yearly` or `annually`              `*-01-01 00:00:00`
`quarterly`                        `*-01,04,07,10-01 00:00:00`
`semiannually` or `semi-annually`  `*-01,07-01 00:00:00`
=================================  ==============================


Here is a table with some useful examples:

======================== =============================  ===================================
Example                  Alternative                    Explanation
======================== =============================  ===================================
`mon,tue,wed,thu,fri`    `mon..fri`                     Every working day at 00:00
`sat,sun`                `sat..sun`                     Only on weekends at 00:00
`mon,wed,fri`            --                             Monday, Wednesday, Friday at 00:00
`12:05`                  --                             Every day at 12:05 PM
`*:00/5`                 `0/1:0/5`                      Every five minutes
`mon..wed *:30/10`       `mon,tue,wed *:30/10`          Monday, Tuesday, Wednesday 30, 40 and 50 minutes after every full hour
`mon..fri 8..17,22:0/15` --                             Every working day every 15 minutes between 8 AM and 6 PM and between 10 PM and 11 PM
`fri 12..13:5/20`        `fri 12,13:5/20`               Friday at 12:05, 12:25, 12:45, 13:05, 13:25 and 13:45
`12,14,16,18,20,22:5`    `12/2:5`                       Every day starting at 12:05 until 22:05, every 2 hours
`*:*`                    `0/1:0/1`                      Every minute (minimum interval)
`*-05`                   --                             On the 5th day of every Month
`Sat *-1..7 15:00`       --                             First Saturday each Month at 15:00
`2015-10-21`             --                             21st October 2015 at 00:00
======================== =============================  ===================================


Differences to systemd
----------------------

Not all features of systemd calendar events are implemented:

* no Unix timestamps (e.g. `@12345`): instead use date and time to specify
  a specific point in time
* no timezone: all schedules use the timezone of the server
* no sub-second resolution
* no reverse day syntax (e.g. 2020-03~01)
* no repetition of ranges (e.g. 1..10/2)

Notes on Scheduling
-------------------

In Proxmox Backup, scheduling for most tasks is done in the
`proxmox-backup-proxy`. This daemon checks all job schedules
every minute, to see if any are due. This means that even though
`calendar events` can contain seconds, it will only be checked
once per minute.

Also, all schedules will be checked against the timezone set
in the Proxmox Backup Server.
