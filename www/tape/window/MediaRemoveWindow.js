Ext.define('PBS.TapeManagement.MediaRemoveWindow', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],

    uuid: undefined,
    label: undefined,

    cbindData: function(config) {
	let me = this;
	return {
	    uuid: me.uuid,
	    warning: Ext.String.format(gettext("Are you sure you want to remove tape '{0}' ?"), me.label),
	};
    },

    title: gettext('Remove Media'),
    url: `/api2/extjs/tape/media/destroy`,

    layout: 'hbox',
    width: 400,
    method: 'GET',
    isCreate: true,
    submitText: gettext('Ok'),
    items: [
	{
	    xtype: 'container',
	    padding: 0,
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },
	    items: [
		{
		    xtype: 'component',
		    cls: [Ext.baseCSSPrefix + 'message-box-icon',
			Ext.baseCSSPrefix + 'message-box-warning',
			Ext.baseCSSPrefix + 'dlg-icon'],
		},
		{
		    xtype: 'container',
		    flex: 1,
		    items: [
			{
			    xtype: 'displayfield',
			    cbind: {
				value: '{warning}',
			    },
			},
			{
			    xtype: 'hidden',
			    name: 'uuid',
			    cbind: {
				value: '{uuid}',
			    },
			},
			{
			    xtype: 'proxmoxcheckbox',
			    fieldLabel: gettext('Force'),
			    name: 'force',
			},
		    ],
		},
	    ],
	},
    ],
});
