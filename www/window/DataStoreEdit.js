Ext.define('PBS.DataStoreEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsDataStoreEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    subject: gettext('Datastore'),
    isAdd: true,

    cbindData: function(initialConfig) {
	var me = this;

	let name = initialConfig.name;
	let baseurl = '/api2/extjs/config/datastore';

	me.isCreate = !name;
	me.url = name ? baseurl + '/' + name : baseurl;
	me.method = name ? 'PUT' : 'POST';
	me.autoLoad = !!name;
	return {};
    },

    items: [
	{
	    xtype: 'inputpanel',
	    column1: [
		{
		    xtype: 'pmxDisplayEditField',
		    cbind: {
			editable: '{isCreate}',
		    },
		    name: 'name',
		    allowBlank: false,
		    fieldLabel: gettext('Name'),
		},
	    ],

	    column2: [
		{
		    xtype: 'pmxDisplayEditField',
		    cbind: {
			editable: '{isCreate}',
		    },
		    name: 'path',
		    allowBlank: false,
		    fieldLabel: gettext('Backing Path'),
		    emptyText: gettext('An absolute path'),
		},
	    ],

	    columnB: [
		{
		    xtype: 'textfield',
		    name: 'comment',
		    fieldLabel: gettext('Comment'),
		},
	    ],
	}
    ],
});
