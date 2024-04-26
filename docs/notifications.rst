.. _notifications:

Notifications
=============

Overview
--------

* Proxmox Backup Server emits :ref:`notification_events` in case of noteworthy
  events in the system. These events are handled by the notification system.
  A notification event has metadata, for example a timestamp, a severity level,
  a type and other metadata fields.
* :ref:`notification_matchers` route a notification event to one or more notification
  targets. A matcher can have match rules to selectively route based on the metadata
  of a notification event.
* :ref:`notification_targets` are a destination to which a notification event
  is routed to by a matcher. There are multiple types of target, mail-based
  (Sendmail and SMTP) and Gotify.

Datastores and tape backup jobs have a configurable :ref:`notification_mode`.
It allows you to choose between the notification system and a legacy mode
for sending notification emails. The legacy mode is equivalent to the
way notifications were handled before Proxmox Backup Server 3.2.

The notification system can be configured in the GUI under
*Configuration → Notifications*. The configuration is stored in
:ref:`notifications.cfg` and :ref:`notifications_priv.cfg` -
the latter contains sensitive configuration options such as
passwords or authentication tokens for notification targets and
can only be read by ``root``.

.. _notification_targets:

Notification Targets
--------------------

Proxmox Backup Server offers multiple types of notification targets.

.. _notification_targets_sendmail:

Sendmail
^^^^^^^^
The sendmail binary is a program commonly found on Unix-like operating systems
that handles the sending of email messages.
It is a command-line utility that allows users and applications to send emails
directly from the command line or from within scripts.

The sendmail notification target uses the ``sendmail`` binary to send emails to a
list of configured users or email addresses. If a user is selected as a recipient,
the email address configured in user's settings will be used.
For the ``root@pam`` user, this is the email address entered during installation.
A user's email address can be configured in ``Configuration -> Access Control -> User Management``.
If a user has no associated email address, no email will be sent.

.. NOTE:: In standard Proxmox Backup Server installations, the ``sendmail`` binary is provided by
   Postfix. It may be necessary to configure Postfix so that it can deliver
   mails correctly - for example by setting an external mail relay (smart host).
   In case of failed delivery, check the system logs for messages logged by
   the Postfix daemon.

See :ref:`notifications.cfg` for all configuration options.

.. _notification_targets_smtp:

SMTP
^^^^
SMTP notification targets can send emails directly to an SMTP mail relay.
This target does not use the system's MTA to deliver emails.
Similar to sendmail targets, if a user is selected as a recipient, the user's configured
email address will be used.

.. NOTE:: Unlike sendmail targets, SMTP targets do not have any queuing/retry mechanism
   in case of a failed mail delivery.

See :ref:`notifications.cfg` for all configuration options.

.. _notification_targets_gotify:

Gotify
^^^^^^
`Gotify <http://gotify.net>`_ is an open-source self-hosted notification server that
allows you to send push notifications to various devices and
applications. It provides a simple API and web interface, making it easy to
integrate with different platforms and services.

See :ref:`notifications.cfg` for all configuration options.

.. _notification_matchers:

Notification Matchers
---------------------

Notification matchers route notifications to notification targets based
on their matching rules. These rules can match certain properties of a
notification, such as the timestamp (``match-calendar``), the severity of
the notification (``match-severity``) or metadata fields (``match-field``).
If a notification is matched by a matcher, all targets configured for the
matcher will receive the notification.

An arbitrary number of matchers can be created, each with with their own
matching rules and targets to notify.
Every target is notified at most once for every notification, even if
the target is used in multiple matchers.

A matcher without rules matches any notification; the configured targets
will always be notified.

See :ref:`notifications.cfg` for all configuration options.

Calendar Matching Rules
^^^^^^^^^^^^^^^^^^^^^^^
A calendar matcher matches a notification's timestamp.

Examples:

* ``match-calendar 8-12``
* ``match-calendar 8:00-15:30``
* ``match-calendar mon-fri 9:00-17:00``
* ``match-calendar sun,tue-wed,fri 9-17``

Field Matching Rules
^^^^^^^^^^^^^^^^^^^^
Notifications have a selection of metadata fields that can be matched.
When using ``exact`` as a matching mode, a ``,`` can be used as a separator.
The matching rule then matches if the metadata field has **any** of the specified
values.

Examples:

* ``match-field exact:type=gc`` Only match notifications for garbage collection jobs
* ``match-field exact:type=prune,verify`` Match prune job and verification job notifications.
* ``match-field regex:datastore=^backup-.*$`` Match any datastore starting with ``backup``.

If a notification does not have the matched field, the rule will **not** match.
For instance, a ``match-field regex:datastore=.*`` directive will match any notification that has
a ``datastore`` metadata field, but will not match if the field does not exist.

Severity Matching Rules
^^^^^^^^^^^^^^^^^^^^^^^
A notification has a associated severity that can be matched.

Examples:

* ``match-severity error``: Only match errors
* ``match-severity warning,error``: Match warnings and error

The following severities are in use:
``info``, ``notice``, ``warning``, ``error``, ``unknown``.

.. _notification_events:

Notification Events
-------------------

The following table contains a list of all notification events in Proxmox Backup server, their
type, severity and additional metadata fields. ``type`` as well as any other metadata field
may be used in ``match-field`` match rules.

================================ ==================== ========== ==============================================================
Event                            ``type``             Severity   Metadata fields (in addition to ``type``)
================================ ==================== ========== ==============================================================
ACME certificate renewal failed  ``acme``             ``error``  ``hostname``
Garbage collection failure       ``gc``               ``error``  ``datastore``, ``hostname``
Garbage collection success       ``gc``               ``info``   ``datastore``, ``hostname``
Package updates available        ``package-updates``  ``info``   ``hostname``
Prune job failure                ``prune``            ``error``  ``datastore``, ``hostname``, ``job-id``
Prune job success                ``prune``            ``info``   ``datastore``, ``hostname``, ``job-id``
Remote sync failure              ``sync``             ``error``  ``datastore``, ``hostname``, ``job-id``
Remote sync success              ``sync``             ``info``   ``datastore``, ``hostname``, ``job-id``
Tape backup job failure          ``tape-backup``      ``error``  ``datastore``, ``hostname``, ``media-pool``, ``job-id``
Tape backup job success          ``tape-backup``      ``info``   ``datastore``, ``hostname``, ``media-pool``, ``job-id``
Tape loading request             ``tape-load``        ``notice`` ``hostname``
Verification job failure         ``verification``     ``error``  ``datastore``, ``hostname``, ``job-id``
Verification job success         ``verification``     ``info``   ``datastore``, ``hostname``, ``job-id``
================================ ==================== ========== ==============================================================

The following table contains a description of all use metadata fields. All of these
can be used in ``match-field`` match rules.

==================== ===================================
Metadata field       Description
==================== ===================================
``datastore``        The name of the datastore
``hostname``         The hostname of the backup server
``job-id``           Job ID
``media-pool``       The name of the tape media pool
``type``             Notification event type
==================== ===================================

.. NOTE:: The daily task checking for any available system updates only sends
   notifications if the node has an active subscription.

System Mail Forwarding
----------------------
Certain local system daemons, such as ``smartd``, send notification emails
to the local ``root`` user. Proxmox Backup Server will feed these mails
into the notification system as a notification of type ``system-mail``
and with severity ``unknown``.

When the email is forwarded to a sendmail target, the mail's content and headers
are forwarded as-is. For all other targets,
the system tries to extract both a subject line and the main text body
from the email content. In instances where emails solely consist of HTML
content, they will be transformed into plain text format during this process.

Permissions
-----------
In order to modify/view the configuration for notification targets,
the ``Sys.Modify/Sys.Audit`` permissions are required for the
``/system/notifications`` ACL node.

.. _notification_mode:

Notification Mode
-----------------
Datastores and tape backup/restore job configuration have a ``notification-mode``
option which can have one of two values:

* ``legacy-sendmail``: Send notification emails via the system's ``sendmail`` command.
  The notification system will be bypassed and any configured targets/matchers will be ignored.
  This mode is equivalent to the notification behavior for version before
  Proxmox Backup Server 3.2.

* ``notification-system``: Use the new, flexible notification system.

If the ``notification-mode`` option is not set, Proxmox Backup Server will default
to ``legacy-sendmail``.

Starting with Proxmox Backup Server 3.2, a datastore created in the UI will
automatically opt in to the new notification system. If the datastore is created
via the API or the ``proxmox-backup-manager`` CLI, the ``notification-mode``
option has to be set explicitly to ``notification-system`` if the
notification system shall be used.

The ``legacy-sendmail`` mode might be removed in a later release of
Proxmox Backup Server.

Settings for ``legacy-sendmail`` notification mode
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

If ``notification-mode`` is set to ``legacy-sendmail``,  Proxmox Backup Server
will send notification emails via the system's ``sendmail`` command to the email
address configured for the user set in the ``notify-user`` option
(falling back to ``root@pam`` if not set).

For datastores, you can also change the level of notifications received per task
type via the ``notify`` option.

* Always: send a notification for any scheduled task, independent of the
  outcome

* Errors: send a notification for any scheduled task that results in an error

* Never: do not send any notification at all

The ``notify-user`` and ``notify`` options are ignored if ``notification-mode``
is set to ``notification-system``.
