Ext.define('PBS.NodeOptionView', {
    extend: 'Proxmox.grid.ObjectGrid',
    alias: 'widget.pbsNodeOptionView',

    monStoreErrors: true,

    url: `/api2/json/nodes/${Proxmox.NodeName}/config`,
    editorConfig: {
	url: `/api2/extjs/nodes/${Proxmox.NodeName}/config`,
    },
    interval: 5000,
    cwidth1: 200,

    listeners: {
	itemdblclick: function() { this.run_editor(); },
	activate: function() { this.rstore.startUpdate(); },
	destroy: function() { this.rstore.stopUpdate(); },
	deactivate: function() { this.rstore.stopUpdate(); },
    },

    tbar: [
	{
	    text: gettext('Edit'),
	    xtype: 'proxmoxButton',
	    disabled: true,
	    handler: btn => btn.up('grid').run_editor(),
	},
    ],

    gridRows: [
	{
	    xtype: 'text',
	    name: 'http-proxy',
	    text: gettext('HTTP proxy'),
	    defaultValue: Proxmox.Utils.noneText,
	    vtype: 'HttpProxy',
	    deleteEmpty: true,
	    onlineHelp: 'node_options_http_proxy',
	},
	{
	    xtype: 'text',
	    name: 'email-from',
	    defaultValue: gettext('root@$hostname'),
	    text: gettext('Email from address'),
	    vtype: 'proxmoxMail',
	    deleteEmpty: true,
	},
	{
	    xtype: 'combobox',
	    name: 'default-lang',
	    text: gettext('Default Language'),
	    defaultValue: '__default__',
	    comboItems: Proxmox.Utils.language_array(),
	    deleteEmpty: true,
	    renderer: Proxmox.Utils.render_language,
	},
    ],
});
