/*global Proxmox*/
Ext.define('PBS.Application', {
    extend: 'Ext.app.Application',

    name: 'PBS',
    appProperty: 'app',

    stores: [
	// 'NavigationStore'
    ],

    layout: 'fit',

    realignWindows: function() {
	var modalwindows = Ext.ComponentQuery.query('window[modal]');
	Ext.Array.forEach(modalwindows, function(item) {
	    item.center();
	});
    },

    logout: function() {
	var me = this;
	//Proxmox.Utils.authClear();
	//me.changeView('loginview', true);
    },

    changeView: function(view, skipCheck) {
	var me = this;
	//?
    },

    launch: function() {
	var me = this;
	Ext.on('resize', me.realignWindows);

	var provider = new Ext.state.LocalStorageProvider({ prefix: 'ext-pbs-' });
	Ext.state.Manager.setProvider(provider);

	// fixme: show login window if not loggedin

	me.currentView = Ext.create({
	    xtype: 'mainview'
	});
    }
});

Ext.application('PBS.Application');
