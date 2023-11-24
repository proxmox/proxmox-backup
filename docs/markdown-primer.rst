.. _markdown-primer:

Markdown Primer
===============

  "Markdown is a text-to-HTML conversion tool for web writers. Markdown allows
  you to write using an easy-to-read, easy-to-write plain text format, then
  convert it to structurally valid XHTML (or HTML)."

  --  John Gruber, https://daringfireball.net/projects/markdown/


The "Notes" panel of the `Proxmox Backup`_ Server web-interface supports
rendering Markdown text.

Proxmox Backup Server supports CommonMark with most extensions of GFM (GitHub
Flavoured Markdown), like tables or task-lists.

.. _markdown_basics:

Markdown Basics
---------------

Note that we only describe the basics here. Please search the web for more
extensive resources, for example on https://www.markdownguide.org/

Headings
~~~~~~~~

.. code-block:: md

  # This is a Heading h1
  ## This is a Heading h2
  ##### This is a Heading h5


Emphasis
~~~~~~~~

Use ``*text*`` or ``_text_`` for emphasis.

Use ``**text**`` or ``__text__`` for bold, heavy-weight text.

Combinations are also possible, for example:

.. code-block:: md

  _You **can** combine them_


Links
~~~~~

You can use automatic detection of links. For example,
``https://forum.proxmox.com/`` would transform it into a clickable link.

You can also control the link text, for example:

.. code-block:: md

  Now, [the part in brackets will be the link text](https://forum.proxmox.com/).

Lists
~~~~~

Unordered Lists
^^^^^^^^^^^^^^^

Use ``*`` or ``-`` for unordered lists, for example:

.. code-block:: md

  * Item 1
  * Item 2
  * Item 2a
  * Item 2b


You can create nested lists by adding indentation.

Ordered Lists
^^^^^^^^^^^^^

.. code-block:: md

  1. Item 1
  1. Item 2
  1. Item 3
    1. Item 3a
    1. Item 3b

NOTE: The integer of ordered lists does not need to be correct, they will be numbered automatically.

Task Lists
^^^^^^^^^^

Task lists use a empty box ``[ ]`` for unfinished tasks and a box with an `X` for finished tasks.

For example:


.. code-block:: md

  - [X] First task already done!
  - [X] Second one too
  - [ ] This one is still to-do
  - [ ] So is this one

Tables
~~~~~~

Tables use the pipe symbol ``|`` to separate columns, and ``-`` to separate the
table header from the table body. In that separation, you can also set the text
alignment, making one column left-, center-, or right-aligned.


.. code-block:: md

  | Left columns  | Right columns |  Some  | More | Cols.| Centering Works Too
  | ------------- |--------------:|--------|------|------|:------------------:|
  | left foo      | right foo     | First  | Row  | Here | >center<           |
  | left bar      | right bar     | Second | Row  | Here | 12345              |
  | left baz      | right baz     | Third  | Row  | Here | Test               |
  | left zab      | right zab     | Fourth | Row  | Here | ☁️☁️☁️              |
  | left rab      | right rab     | And    | Last | Here | The End            |

Note that you do not need to align the columns nicely with white space, but that makes
editing tables easier.

Block Quotes
~~~~~~~~~~~~

You can enter block quotes by prefixing a line with ``>``, similar as in plain-text emails.

.. code-block:: md

  > Markdown is a lightweight markup language with plain-text-formatting syntax,
  > created in 2004 by John Gruber with Aaron Swartz.
  >
  >> Markdown is often used to format readme files, for writing messages in online discussion forums,
  >> and to create rich text using a plain text editor.

Code and Snippets
~~~~~~~~~~~~~~~~~

You can use backticks to avoid processing a group of words or paragraphs. This
is useful for preventing a code or configuration hunk from being mistakenly
interpreted as markdown.

Inline Code
^^^^^^^^^^^

Surrounding part of a line with single backticks allows you to write code
inline, for examples:

.. code-block:: md

  This hosts IP address is `10.0.0.1`.

Entire Blocks of Code
^^^^^^^^^^^^^^^^^^^^^

For code blocks spanning several lines, you can use triple-backticks to start
and end such a block, for example:

.. code-block:: md

  ```
  # This is the network config I want to remember here
  auto vmbr2
  iface vmbr2 inet static
          address 10.0.0.1/24
          bridge-ports ens20
          bridge-stp off
          bridge-fd 0
          bridge-vlan-aware yes
          bridge-vids 2-4094

  ```
