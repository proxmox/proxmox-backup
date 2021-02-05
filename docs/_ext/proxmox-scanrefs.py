#!/usr/bin/env python3

# debugging stuff
from pprint import pprint

from typing import cast

import json
import re

import os
import io
from docutils import nodes

from sphinx.builders import Builder
from sphinx.util import logging

logger = logging.getLogger(__name__)

# refs are added in the following manner before the title of a section (note underscore and newline before title):
# .. _my-label:
#
# Section to ref
# --------------
#
#
# then referred to like (note missing underscore):
# "see :ref:`my-label`"
#
# the benefit of using this is if a label is explicitly set for a section,
# we can refer to it with this anchor #my-label in the html,
# even if the section name changes.
#
# see https://www.sphinx-doc.org/en/master/usage/restructuredtext/roles.html#role-ref

def scan_extjs_files(wwwdir="../www"): # a bit rough i know, but we can optimize later
    js_files = []
    used_anchors = []
    logger.info("scanning extjs files for onlineHelp definitions")
    for root, dirs, files in os.walk("{}".format(wwwdir)):
        #print(root, dirs, files)
        for filename in files:
            if filename.endswith('.js'):
                js_files.append(os.path.join(root, filename))
    for js_file in js_files:
        fd = open(js_file).read()
        allmatch = re.findall("(?:onlineHelp:|get_help_tool\s*\()\s*[\'\"](.*?)[\'\"]", fd, re.M)
        for match in allmatch:
            anchor = match
            anchor = re.sub('_', '-', anchor) # normalize labels
            logger.info("found onlineHelp: {} in {}".format(anchor, js_file))
            used_anchors.append(anchor)

    return used_anchors


def setup(app):
    logger.info('Mapping reference labels...')
    app.add_builder(ReflabelMapper)
    return {
        'version': '0.1',
        'parallel_read_safe': True,
        'parallel_write_safe': True,
    }

class ReflabelMapper(Builder):
    name = 'proxmox-scanrefs'

    def init(self):
        self.docnames = []
        self.env.online_help = {}
        self.env.online_help['pbs_documentation_index'] = {
            'link': '/docs/index.html',
            'title': 'Proxmox Backup Server Documentation Index',
        }
        # Disabled until we find a sensible way to scan proxmox-widget-toolkit
        # as well
        #self.env.used_anchors = scan_extjs_files()

        if not os.path.isdir(self.outdir):
            os.mkdir(self.outdir)

        self.output_filename = os.path.join(self.outdir, 'OnlineHelpInfo.js')
        self.output = io.open(self.output_filename, 'w', encoding='UTF-8')

    def write_doc(self, docname, doctree):
            for node in doctree.traverse(nodes.section):
                #pprint(vars(node))

                if hasattr(node, 'expect_referenced_by_id') and len(node['ids']) > 1: # explicit labels
                    filename = self.env.doc2path(docname)
                    filename_html = re.sub('.rst', '.html', filename)

                    # node['ids'][0] contains a normalized version of the
                    # headline.  If the ref and headline are the same
                    # (normalized) sphinx will set the node['ids'][1] to a
                    # generic id in the format `idX` where X is numeric. If the
                    # ref and headline are not the same, the ref name will be
                    # stored in node['ids'][1]
                    if re.match('^id[0-9]*$', node['ids'][1]):
                        labelid = node['ids'][0]
                    else:
                        labelid = node['ids'][1]

                    title = cast(nodes.title, node[0])
                    logger.info('traversing section {}'.format(title.astext()))
                    ref_name = getattr(title, 'rawsource', title.astext())

                    if (ref_name[:7] == ':term:`'):
                        ref_name = ref_name[7:-1]

                    self.env.online_help[labelid] = {'link': '', 'title': ''}
                    self.env.online_help[labelid]['link'] = "/docs/" + os.path.basename(filename_html) + "#{}".format(labelid)
                    self.env.online_help[labelid]['title'] = ref_name

            return


    def get_outdated_docs(self):
        return 'all documents'

    def prepare_writing(self, docnames):
        return

    def get_target_uri(self, docname, typ=None):
        return ''

    def validate_anchors(self):
        #pprint(self.env.online_help)
        to_remove = []

        # Disabled until we find a sensible way to scan proxmox-widget-toolkit
        # as well
        #for anchor in self.env.used_anchors:
        #    if anchor not in self.env.online_help:
        #        logger.info("[-] anchor {} is missing from onlinehelp!".format(anchor))
        #for anchor in self.env.online_help:
        #    if anchor not in self.env.used_anchors and anchor != 'pbs_documentation_index':
        #        logger.info("[*] anchor {} not used! deleting...".format(anchor))
        #        to_remove.append(anchor)
        #for anchor in to_remove:
        #   self.env.online_help.pop(anchor, None)
        return

    def finish(self):
        # generate OnlineHelpInfo.js output
        self.validate_anchors()

        self.output.write("const proxmoxOnlineHelpInfo = ")
        self.output.write(json.dumps(self.env.online_help, indent=2))
        self.output.write(";\n")
        self.output.close()
        return
