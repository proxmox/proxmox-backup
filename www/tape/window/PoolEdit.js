Ext.define('PBS.TapeManagement.PoolEditWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsPoolEditWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'tape_media_pool_config',

    isCreate: true,
    isAdd: true,
    subject: gettext('Media Pool'),
    cbindData: function(initialConfig) {
	let me = this;

	let poolid = initialConfig.poolid;
	let baseurl = '/api2/extjs/config/media-pool';

	me.isCreate = !poolid;
	me.url = poolid ? `${baseurl}/${encodeURIComponent(poolid)}` : baseurl;
	me.method = poolid ? 'PUT' : 'POST';

	return { };
    },

    items: {
	xtype: 'inputpanel',
	column1: [
	    {
		fieldLabel: gettext('Name'),
		name: 'name',
		xtype: 'pmxDisplayEditField',
		renderer: Ext.htmlEncode,
		allowBlank: false,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Allocation Policy'),
		xtype: 'pbsAllocationSelector',
		name: 'allocation',
		skipEmptyText: true,
		allowBlank: true,
		autoSelect: false,
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Retention Policy'),
		xtype: 'pbsRetentionSelector',
		name: 'retention',
		skipEmptyText: true,
		allowBlank: true,
		autoSelect: false,
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Encryption Key'),
		xtype: 'pbsTapeKeySelector',
		name: 'encrypt',
		allowBlank: true,
		skipEmptyText: true,
		autoSelect: false,
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	columnB: [
	    {
		fieldLabel: gettext('Comment'),
		xtype: 'proxmoxtextfield',
		name: 'comment',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],
    },
});
