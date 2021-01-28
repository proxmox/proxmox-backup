Ext.define('PBS.TapeManagement.LabelMediaWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsLabelMediaWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    title: gettext('Label Media'),
    submitText: gettext('OK'),

    showProgress: true,

    items: [
	{
	    xtype: 'displayfield',
	    fieldLabel: gettext('Drive'),
	    cbind: {
		value: '{driveid}',
	    },
	},
	{
	    fieldLabel: gettext('Label'),
	    name: 'label-text',
	    xtype: 'proxmoxtextfield',
	    allowBlank: false,
	},
	{
	    xtype: 'pbsMediaPoolSelector',
	    fieldLabel: gettext('Media Pool'),
	    name: 'pool',
	    allowBlank: true,
	    skipEmptyText: true,
	},
    ],

    initComponent: function() {
	let me = this;
	if (!me.driveid) {
	    throw "no driveid given";
	}

	let driveid = encodeURIComponent(me.driveid);
	me.url = `/api2/extjs/tape/drive/${driveid}/label-media`;
	me.callParent();
    },
});

