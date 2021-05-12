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
	itemdblclick: function() { this.run_editor() },
    },

    tbar: [
	{
	    text: gettext('Edit'),
	    xtype: 'proxmoxButton',
	    disabled: true,
	    handler: function() { this.up('grid').run_editor(); },
	}
    ],

    initComponent: function() {
	let me = this;

	me.add_text_row('http-proxy', gettext('HTTP proxy'), {
	    defaultValue: Proxmox.Utils.noneText,
	    vtype: 'HttpProxy',
	    deleteEmpty: true,
	});

	me.callParent();

	me.on('activate', me.rstore.startUpdate);
	me.on('destroy', me.rstore.stopUpdate);
	me.on('deactivate', me.rstore.stopUpdate);
    },
});
